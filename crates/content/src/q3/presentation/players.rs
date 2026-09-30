//! Quake III presentation: players.
//!
//! Donor provenance: `src/content/q3/presentation/players.ts`.

use qa_core::math::{
    add3, angles_to_axis, cross3, dot3, length3, normalize3, scale3, sub3, vec3, vec4, Axis, Vec3, Vec4,
};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::format::{game_format_bounded, GameFormatArgument};
use crate::q3::base::shared::definitions::*;
use crate::q3::base::shared::entity_state::EntityState as CanonicalEntityState;
use crate::q3::base::shared::player_state::PlayerState as CanonicalPlayerState;
use crate::q3::base::shared::trajectory::evaluate_trajectory;
use crate::q3::base::world::{TraceContact, TraceSolidity};
use crate::q3::foundation::animation::ANIMATION_TOGGLE_BIT;
use crate::q3::presentation::client_info::ClientInfo;
use crate::q3::presentation::collision_host::*;
use crate::q3::presentation::ref_entity::{RF_LIGHTING_ORIGIN, RF_SHADOW_PLANE, RF_THIRD_PERSON};
use crate::q3::presentation::resources::*;
use crate::q3::presentation::retail_snapshot::*;
use crate::q3::presentation::state::*;

// ---------------------------------------------------------------------------
// Player media and presentation (players.ts)
// ---------------------------------------------------------------------------

/// Copy a client model between slots (`CG_CopyClientInfoModel`).
pub fn copy_client_model(from: &ClientInfo, to: &mut ClientInfo) -> PresentResult<()> {
    to.head_offset = from.head_offset;
    to.footsteps = from.footsteps;
    to.gender = from.gender;
    to.legs_model = from.legs_model.clone();
    to.legs_skin = from.legs_skin.clone();
    to.torso_model = from.torso_model.clone();
    to.torso_skin = from.torso_skin.clone();
    to.head_model = from.head_model.clone();
    to.head_skin = from.head_skin.clone();
    to.model_icon = from.model_icon.clone();
    to.new_anims = from.new_anims;
    to.animations = from.animations;
    to.sounds = from.sounds;
    Ok(())
}

/// Custom player sound names (`CUSTOM_SOUND_NAMES`).
pub const CUSTOM_SOUND_NAMES: [&str; 13] = [
    "*death1.wav",
    "*death2.wav",
    "*death3.wav",
    "*jump1.wav",
    "*pain25_1.wav",
    "*pain50_1.wav",
    "*pain75_1.wav",
    "*pain100_1.wav",
    "*falling1.wav",
    "*gasp.wav",
    "*drown.wav",
    "*fall1.wav",
    "*taunt.wav",
];

// Expanded from q3_int_enum! so players.rs needs no cross-module macro import.
/// Player animation number (`PlayerAnimation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum PlayerAnim {
    /// Both death 1.
    BothDeath1 = 0,
    /// Both dead 1.
    BothDead1 = 1,
    /// Both death 2.
    BothDeath2 = 2,
    /// Both dead 2.
    BothDead2 = 3,
    /// Both death 3.
    BothDeath3 = 4,
    /// Both dead 3.
    BothDead3 = 5,
    /// Torso gesture.
    TorsoGesture = 6,
    /// Torso attack.
    TorsoAttack = 7,
    /// Torso attack 2.
    TorsoAttack2 = 8,
    /// Torso drop.
    TorsoDrop = 9,
    /// Torso raise.
    TorsoRaise = 10,
    /// Torso stand.
    TorsoStand = 11,
    /// Torso stand 2.
    TorsoStand2 = 12,
    /// Legs walk crouch.
    LegsWalkcr = 13,
    /// Legs walk.
    LegsWalk = 14,
    /// Legs run.
    LegsRun = 15,
    /// Legs back.
    LegsBack = 16,
    /// Legs swim.
    LegsSwim = 17,
    /// Legs jump.
    LegsJump = 18,
    /// Legs land.
    LegsLand = 19,
    /// Legs jump back.
    LegsJumpb = 20,
    /// Legs land back.
    LegsLandb = 21,
    /// Legs idle.
    LegsIdle = 22,
    /// Legs idle crouch.
    LegsIdlecr = 23,
    /// Legs turn.
    LegsTurn = 24,
    /// Torso get flag.
    TorsoGetflag = 25,
    /// Torso guard base.
    TorsoGuardbase = 26,
    /// Torso patrol.
    TorsoPatrol = 27,
    /// Torso follow me.
    TorsoFollowme = 28,
    /// Torso affirmative.
    TorsoAffirmative = 29,
    /// Torso negative.
    TorsoNegative = 30,
    /// Legs back crouch.
    LegsBackcr = 32,
    /// Legs back walk.
    LegsBackwalk = 33,
    /// Flag run.
    FlagRun = 34,
    /// Flag stand.
    FlagStand = 35,
    /// Flag stand to run.
    FlagStand2run = 36,
}

impl PlayerAnim {
    /// Raw source value lookup.
    #[must_use]
    pub const fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::BothDeath1),
            1 => Some(Self::BothDead1),
            2 => Some(Self::BothDeath2),
            3 => Some(Self::BothDead2),
            4 => Some(Self::BothDeath3),
            5 => Some(Self::BothDead3),
            6 => Some(Self::TorsoGesture),
            7 => Some(Self::TorsoAttack),
            8 => Some(Self::TorsoAttack2),
            9 => Some(Self::TorsoDrop),
            10 => Some(Self::TorsoRaise),
            11 => Some(Self::TorsoStand),
            12 => Some(Self::TorsoStand2),
            13 => Some(Self::LegsWalkcr),
            14 => Some(Self::LegsWalk),
            15 => Some(Self::LegsRun),
            16 => Some(Self::LegsBack),
            17 => Some(Self::LegsSwim),
            18 => Some(Self::LegsJump),
            19 => Some(Self::LegsLand),
            20 => Some(Self::LegsJumpb),
            21 => Some(Self::LegsLandb),
            22 => Some(Self::LegsIdle),
            23 => Some(Self::LegsIdlecr),
            24 => Some(Self::LegsTurn),
            25 => Some(Self::TorsoGetflag),
            26 => Some(Self::TorsoGuardbase),
            27 => Some(Self::TorsoPatrol),
            28 => Some(Self::TorsoFollowme),
            29 => Some(Self::TorsoAffirmative),
            30 => Some(Self::TorsoNegative),
            32 => Some(Self::LegsBackcr),
            33 => Some(Self::LegsBackwalk),
            34 => Some(Self::FlagRun),
            35 => Some(Self::FlagStand),
            36 => Some(Self::FlagStand2run),
            _ => None,
        }
    }
}

/// Player solid mask.
pub const MASK_PLAYER_SOLID: i32 = 1 | 0x10000 | 0x2000000;

/// Water contents mask.
pub const MASK_WATER: i32 = 8 | 16 | 32;

/// Dead entity flag.
pub const PLAYER_DEAD: i32 = 1;

/// Kamikaze entity flag.
pub const PLAYER_KAMIKAZE: i32 = 0x200;

/// Client info settings (`ClientInfoSettings`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfoSettings {
    /// Game type.
    pub game_type: GameType,
    /// Maximum clients.
    pub max_clients: i32,
    /// Forced models.
    pub force_model: bool,
    /// Forced model.
    pub model: String,
    /// Forced head model.
    pub head_model: String,
    /// Mission-pack red team name.
    pub red_team_name: String,
    /// Mission-pack blue team name.
    pub blue_team_name: String,
    /// Deferred player models.
    pub defer_players: bool,
    /// Build-script mode.
    pub build_script: bool,
    /// Loading screen active.
    pub loading: bool,
}

/// Client info host (`ClientInfoHost`).
///
/// The donor's `state` product, animation-config parser, and tag lookup fold
/// into direct methods; donor `async` registration is synchronous here.
pub trait ClientInfoHost: AssetReader {
    /// Presentation product.
    fn product(&self) -> Product;
    /// Current settings.
    fn settings(&self) -> ClientInfoSettings;
    /// Bytes of memory remaining.
    fn memory_remaining(&self) -> i64;
    /// Register a player model.
    fn register_model(&mut self, path: &str) -> PresentResult<SceneModel>;
    /// Register a player skin.
    fn register_skin(&mut self, path: &str) -> PresentResult<Option<SceneSkin>>;
    /// Register an unmipmapped icon shader.
    fn register_shader_no_mip(&mut self, name: &str) -> PresentResult<Option<SceneShader>>;
    /// Register a player sound.
    fn register_sound(&mut self, name: &str) -> PresentResult<Option<PcmSound>>;
    /// Fetch a preloaded sound without registering.
    fn sound(&self, name: &str) -> Option<PcmSound>;
    /// Parse an `animation.cfg` into a slot; `false` is a parse failure.
    fn parse_animation_config(&mut self, ci: &mut ClientInfo, text: &str, path: &str) -> PresentResult<bool>;
    /// Whether a model carries an animation tag.
    fn model_has_tag(&mut self, model: &SceneModel, tag: &str) -> bool;
    /// Diagnostic print.
    fn print(&mut self, message: &str);
}

/// Player media (`PlayerMedia`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerMedia {
    /// Connection shader.
    pub connection_shader: Option<SceneShader>,
    /// Balloon shader.
    pub balloon_shader: Option<SceneShader>,
    /// Impressive medal.
    pub medal_impressive: Option<SceneShader>,
    /// Excellent medal.
    pub medal_excellent: Option<SceneShader>,
    /// Gauntlet medal.
    pub medal_gauntlet: Option<SceneShader>,
    /// Defend medal.
    pub medal_defend: Option<SceneShader>,
    /// Assist medal.
    pub medal_assist: Option<SceneShader>,
    /// Capture medal.
    pub medal_capture: Option<SceneShader>,
    /// Friend shader.
    pub friend_shader: Option<SceneShader>,
    /// Shadow mark shader.
    pub shadow_mark_shader: Option<SceneShader>,
    /// Wake mark shader.
    pub wake_mark_shader: Option<SceneShader>,
    /// Invisibility shader.
    pub invis_shader: Option<SceneShader>,
    /// Quad shader.
    pub quad_shader: Option<SceneShader>,
    /// Red quad shader.
    pub red_quad_shader: Option<SceneShader>,
    /// Regen shader.
    pub regen_shader: Option<SceneShader>,
    /// Battlesuit shader.
    pub battle_suit_shader: Option<SceneShader>,
    /// Haste puff shader.
    pub haste_puff_shader: Option<SceneShader>,
    /// Flight loop sound.
    pub flight_sound: Option<PcmSound>,
    /// Red flag model.
    pub red_flag_model: SceneModel,
    /// Blue flag model.
    pub blue_flag_model: SceneModel,
    /// Neutral flag model.
    pub neutral_flag_model: SceneModel,
    /// Flag pole model.
    pub flag_pole_model: SceneModel,
    /// Flag flap model.
    pub flag_flap_model: SceneModel,
    /// Red flag flap skin.
    pub red_flag_flap_skin: Option<SceneSkin>,
    /// Blue flag flap skin.
    pub blue_flag_flap_skin: Option<SceneSkin>,
    /// Neutral flag flap skin.
    pub neutral_flag_flap_skin: Option<SceneSkin>,
}

/// Mission-pack player media (`MissionPlayerMedia`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissionPlayerMedia {
    /// Red harvester cube.
    pub red_cube_model: SceneModel,
    /// Blue harvester cube.
    pub blue_cube_model: SceneModel,
    /// Kamikaze head model.
    pub kamikaze_head_model: SceneModel,
    /// Kamikaze head trail.
    pub kamikaze_head_trail: SceneModel,
    /// Guard powerup model.
    pub guard_powerup_model: SceneModel,
    /// Scout powerup model.
    pub scout_powerup_model: SceneModel,
    /// Doubler powerup model.
    pub doubler_powerup_model: SceneModel,
    /// Ammo-regen powerup model.
    pub ammo_regen_powerup_model: SceneModel,
    /// Invulnerability powerup model.
    pub invulnerability_powerup_model: SceneModel,
    /// Medkit usage model.
    pub medkit_usage_model: SceneModel,
    /// Shotgun smoke puff shader.
    pub shotgun_smoke_puff_shader: Option<SceneShader>,
    /// Dust puff shader.
    pub dust_puff_shader: Option<SceneShader>,
}

/// Player presentation settings (`PlayerPresentationSettings`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerPresentationSettings {
    /// Game type.
    pub game_type: GameType,
    /// Camera mode.
    pub camera_mode: bool,
    /// Frozen player animations.
    pub no_player_animations: bool,
    /// Animation speed.
    pub animation_speed: f32,
    /// Swing speed.
    pub swing_speed: f32,
    /// Draw friend markers.
    pub draw_friend: bool,
    /// Shadow mode.
    pub shadows: i32,
    /// Breath puffs.
    pub enable_breath: bool,
    /// Dust puffs.
    pub enable_dust: bool,
    /// Debug position prints.
    pub debug_position: bool,
    /// Debug animation prints.
    pub debug_animation: bool,
}

/// Lit player polygon vertex (`PlayerPolyVertex`).
pub type PlayerPolyVertex = RefPolyVertex;

/// QVM body part (`QvmBodyPart`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmBodyPart {
    /// Whole body.
    Body,
    /// Lower body.
    Lower,
    /// Upper body.
    Upper,
    /// Head.
    Head,
}

impl QvmBodyPart {
    /// Source part name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Lower => "lower",
            Self::Upper => "upper",
            Self::Head => "head",
        }
    }
}

/// Smoke puff request (`SmokePuffOptions`).
#[derive(Debug, Clone, PartialEq)]
pub struct SmokePuffOptions {
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Radius.
    pub radius: f32,
    /// Color.
    pub color: Vec4,
    /// Duration in milliseconds.
    pub duration: i32,
    /// Start time.
    pub start_time: i32,
    /// Fade-in time.
    pub fade_in_time: i32,
    /// Local-entity flags.
    pub flags: i32,
    /// Puff shader.
    pub shader: Option<SceneShader>,
}

/// Impact mark request (`ImpactMarkRequest`).
#[derive(Debug, Clone, PartialEq)]
pub struct ImpactMarkRequest {
    /// Mark shader.
    pub shader: Option<SceneShader>,
    /// Origin.
    pub origin: Vec3,
    /// Direction.
    pub direction: Vec3,
    /// Orientation in degrees.
    pub orientation: f32,
    /// Color.
    pub color: Vec4,
    /// Alpha fade.
    pub alpha_fade: bool,
    /// Radius.
    pub radius: f32,
    /// Temporary mark.
    pub temporary: bool,
}

/// Posed body axes (`PlayerPose` legs/torso/head).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerPoseAxes {
    /// Legs axis.
    pub legs: Axis,
    /// Torso axis.
    pub torso: Axis,
    /// Head axis.
    pub head: Axis,
}

/// Player presentation host (`PlayerPresentationHost`).
///
/// Animation stepping, posing, tag placement, effects, marks, and the random
/// stream arrive through these methods; donor `async` boundaries are
/// synchronous here.
pub trait PlayerPresentationHost {
    /// Presentation product.
    fn product(&self) -> Product;
    /// Player media.
    fn media(&self) -> PlayerMedia;
    /// Mission-pack player media.
    fn mission_media(&self) -> Option<MissionPlayerMedia>;
    /// Current settings.
    fn settings(&self) -> PlayerPresentationSettings;
    /// Whether a body is hidden by the recipe.
    fn body_hidden(&mut self, entity: i32) -> bool {
        let _ = entity;
        false
    }
    /// Recipe body submission hook; `true` consumes the submission.
    fn body_submission(&mut self, entity: i32, part: QvmBodyPart, source: &RefModelEntity, base: bool) -> bool {
        let _ = (entity, part, source, base);
        false
    }
    /// Trace against the collision world.
    fn trace_world(&mut self, start: Vec3, end: Vec3, mins: Vec3, maxs: Vec3, mask: i32) -> TraceResult;
    /// Trace against the world, skipping one entity.
    fn trace_skip(
        &mut self,
        start: Vec3,
        end: Vec3,
        mins: Vec3,
        maxs: Vec3,
        skip_number: i32,
        mask: i32,
    ) -> TraceResult;
    /// Contents at a point.
    fn point_contents(&mut self, point: Vec3) -> i32;
    /// Submit a scene entity.
    fn add_entity(&mut self, entity: RefEntity);
    /// Submit a dynamic light.
    fn add_light(&mut self, light: DynamicLight);
    /// Submit a scene polygon.
    fn add_poly(&mut self, poly: RefPoly);
    /// Lighting sample for a point.
    fn light_for_point(&mut self, point: Vec3) -> LightingSample;
    /// Build impact-mark polygons.
    fn impact_mark(&mut self, request: &ImpactMarkRequest) -> Vec<RefPoly>;
    /// Spawn a smoke puff; `scale_fade` selects the shrinking local entity.
    fn smoke_puff(&mut self, options: &SmokePuffOptions, scale_fade: bool);
    /// Add a looping sound.
    fn add_looping_sound(&mut self, entity_num: i32, origin: Vec3, velocity: Vec3, sound: Option<PcmSound>);
    /// Add a player's world weapon.
    fn add_player_weapon(
        &mut self,
        parent: &RefModelEntity,
        ps: Option<&CanonicalPlayerState>,
        entity: &ClientEntity,
        team: Team,
    );
    /// Random integer.
    fn random_int(&mut self) -> i32;
    /// Calculate posed body axes.
    #[allow(clippy::too_many_arguments)]
    fn calculate_pose(
        &mut self,
        player: &ClientPlayerEntity,
        current: &CanonicalEntityState,
        ci: &ClientInfo,
        lerp_angles: Vec3,
        time_ms: i32,
        frame_time_ms: i32,
        swing_speed: f32,
    ) -> PlayerPoseAxes;
    /// Clear a lerp frame.
    fn clear_lerp_frame(&mut self, ci: &ClientInfo, frame: &mut LerpFrame, animation: i32, time_ms: i32, verbose: bool);
    /// Step a lerp frame.
    #[allow(clippy::too_many_arguments)]
    fn run_lerp_frame(
        &mut self,
        ci: &ClientInfo,
        frame: &mut LerpFrame,
        new_animation: i32,
        speed_scale: f32,
        time_ms: i32,
        frozen: bool,
        verbose: bool,
    );
    /// Swing an angle; returns `(angle, swinging)`.
    #[allow(clippy::too_many_arguments)]
    fn swing_angles(
        &mut self,
        destination: f32,
        swing_tolerance: f32,
        clamp_tolerance: f32,
        speed: f32,
        frame_time_ms: i32,
        angle: f32,
        swinging: bool,
    ) -> (f32, bool);
    /// Place an entity on a parent tag.
    fn position_on_tag(
        &mut self,
        entity: &mut RefModelEntity,
        parent: &RefModelEntity,
        parent_model: &SceneModel,
        tag: &str,
    ) -> PresentResult<()>;
    /// Place a rotated entity on a parent tag.
    fn position_rotated_on_tag(
        &mut self,
        entity: &mut RefModelEntity,
        parent: &RefModelEntity,
        parent_model: &SceneModel,
        tag: &str,
    ) -> PresentResult<()>;
    /// Diagnostic print.
    fn print(&mut self, message: &str);
}

/// Player render context: the frame inputs `player` reads and writes.
pub struct PlayerRenderContext<'a> {
    /// Frame time in milliseconds.
    pub time: i32,
    /// Frame duration in milliseconds.
    pub frame_time: i32,
    /// Third-person rendering.
    pub rendering_third_person: bool,
    /// Snapshot player client number.
    pub snapshot_client_num: i32,
    /// Snapshot team.
    pub snapshot_team: i32,
    /// Client info slots.
    pub clients: &'a mut [ClientInfo],
    /// Skull trails.
    pub skull_trails: &'a mut [SkullTrail; MAX_CLIENTS],
}

/// Player shadow outcome.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowOutcome {
    /// Shadow visible.
    pub visible: bool,
    /// Shadow plane height.
    pub plane: f32,
}

pub(crate) fn powered(state: &CanonicalEntityState, powerup: Powerup) -> bool {
    (state.powerups & (1 << (powerup as i32))) != 0
}

pub(crate) fn sphere_angle(time: i32, divisor: i32) -> f32 {
    (((time / divisor) & 255) as f32 * (std::f32::consts::PI * 2.0)) / 255.0
}

pub(crate) fn qpath_truncate(value: &str) -> String {
    value.chars().take(63).collect()
}

pub(crate) fn same_fold(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

pub(crate) fn info_color(value: &str) -> Vec3 {
    let bits = game_atoi(value).unwrap_or_default();
    if !(1..=7).contains(&bits) {
        return vec3(1.0, 1.0, 1.0);
    }
    vec3(
        if bits & 4 != 0 { 1.0 } else { 0.0 },
        if bits & 2 != 0 { 1.0 } else { 0.0 },
        if bits & 1 != 0 { 1.0 } else { 0.0 },
    )
}

pub(crate) fn model_skin(value: &str) -> (String, String) {
    let path = qpath_truncate(value);
    match path.find('/') {
        None => (path, "default".to_string()),
        Some(slash) => (path[..slash].to_string(), path[slash + 1..].to_string()),
    }
}

/// Custom sound fallback model (`q3CustomSoundFallback`).
#[must_use]
pub const fn custom_sound_fallback(product: Product, team_game: bool) -> &'static str {
    match (product, team_game) {
        (Product::Missionpack, true) => "james",
        _ => "sarge",
    }
}

/// Player presenter (`PlayerPresenter`).
pub struct PlayerPresenter<H> {
    /// Host services.
    pub host: H,
}

impl<H: PlayerPresentationHost> PlayerPresenter<H> {
    /// New presenter for one seat's product.
    pub fn new(host: H, product: Product) -> PresentResult<Self> {
        if host.product() != product {
            return Err(state_msg("Player media product differs from cgame product"));
        }
        Ok(Self { host })
    }

    /// Reset a player entity's interpolation.
    pub fn reset_player_entity(&mut self, time: i32, entity: &mut ClientEntity, ci: &ClientInfo) -> PresentResult<()> {
        let settings = self.host.settings();
        entity.error_time = -99999;
        entity.extrapolated = false;
        let legs_anim = entity.current_state.legs_anim;
        self.host.clear_lerp_frame(
            ci,
            &mut entity.player.legs.base,
            legs_anim,
            time,
            settings.debug_animation,
        );
        let torso_anim = entity.current_state.torso_anim;
        self.host.clear_lerp_frame(
            ci,
            &mut entity.player.torso.base,
            torso_anim,
            time,
            settings.debug_animation,
        );
        entity.lerp_origin = evaluate_trajectory(&entity.current_state.pos, time);
        entity.lerp_angles = evaluate_trajectory(&entity.current_state.apos, time);
        entity.raw_origin = entity.lerp_origin;
        entity.raw_angles = entity.lerp_angles;
        // The source memset follows ClearLerpFrame and discards its timing/animation pointer.
        let (yaw, pitch) = (entity.raw_angles.y, entity.raw_angles.x);
        entity.player.legs = PoseLerpFrame {
            base: create_lerp_frame(),
            yaw_angle: yaw,
            yawing: false,
            pitch_angle: 0.0,
            pitching: false,
        };
        entity.player.torso = PoseLerpFrame {
            base: create_lerp_frame(),
            yaw_angle: yaw,
            yawing: false,
            pitch_angle: pitch,
            pitching: false,
        };
        if settings.debug_position {
            let message = game_format_bounded(
                "%i ResetPlayerEntity yaw=%i\n",
                &[
                    GameFormatArgument::from(entity.current_state.number),
                    GameFormatArgument::from(entity.player.torso.yaw_angle.to_bits() as i32),
                ],
                1024,
            );
            self.host.print(&message);
        }
        Ok(())
    }

    /// Submit a body part with its powerup shells.
    pub fn add_ref_entity_with_powerups(
        &mut self,
        entity: &mut RefModelEntity,
        state: &CanonicalEntityState,
        team: Team,
        part: Option<QvmBodyPart>,
        time: i32,
    ) {
        if self.host.body_hidden(state.number) {
            return;
        }
        let media = self.host.media();
        let initial_shader = entity.custom_shader.clone();
        let submit = |host: &mut H, entity: &RefModelEntity| {
            let consumed = match part {
                None => false,
                Some(part) => host.body_submission(state.number, part, entity, entity.custom_shader == initial_shader),
            };
            if !consumed {
                host.add_entity(RefEntity::Model(entity.clone()));
            }
        };
        if powered(state, Powerup::PwInvis) {
            entity.custom_shader = media.invis_shader.clone();
            submit(&mut self.host, entity);
            return;
        }
        submit(&mut self.host, entity);
        if powered(state, Powerup::PwQuad) {
            entity.custom_shader = if team == Team::TeamRed {
                media.red_quad_shader.clone()
            } else {
                media.quad_shader.clone()
            };
            submit(&mut self.host, entity);
        }
        if powered(state, Powerup::PwRegen) && (time / 100) % 10 == 1 {
            entity.custom_shader = media.regen_shader.clone();
            submit(&mut self.host, entity);
        }
        if powered(state, Powerup::PwBattlesuit) {
            entity.custom_shader = media.battle_suit_shader.clone();
            submit(&mut self.host, entity);
        }
    }

    /// Light player polygon vertices.
    pub fn light_verts(&mut self, normal: Vec3, vertices: &mut [PlayerPolyVertex]) -> PresentResult<bool> {
        let Some(first) = vertices.first() else {
            return Err(state_msg("Player light vertices require at least one vertex"));
        };
        let light = self.host.light_for_point(first.position);
        let incoming = dot3(normal, light.light_dir);
        for vertex in vertices.iter_mut() {
            let component = |ambient: f32, directed: f32| -> f32 {
                if incoming <= 0.0 {
                    ((ambient as i32) & 255) as f32
                } else {
                    (((ambient + incoming * directed) as i32).min(255) & 255) as f32
                }
            };
            vertex.color = vec4(
                component(light.ambient_light.x, light.directed_light.x),
                component(light.ambient_light.y, light.directed_light.y),
                component(light.ambient_light.z, light.directed_light.z),
                255.0,
            );
        }
        Ok(true)
    }

    fn animate(
        &mut self,
        entity: &mut ClientEntity,
        ci: &ClientInfo,
        legs: &mut RefModelEntity,
        torso: &mut RefModelEntity,
        options: &PlayerPresentationSettings,
        time: i32,
    ) {
        if options.no_player_animations {
            return;
        }
        let state = entity.current_state.clone();
        let speed_scale = if powered(&state, Powerup::PwHaste) { 1.5 } else { 1.0 };
        let animation =
            if entity.player.legs.yawing && (state.legs_anim & !ANIMATION_TOGGLE_BIT) == PlayerAnim::LegsIdle as i32 {
                PlayerAnim::LegsTurn as i32
            } else {
                state.legs_anim
            };
        let frozen = options.animation_speed == 0.0;
        self.host.run_lerp_frame(
            ci,
            &mut entity.player.legs.base,
            animation,
            speed_scale,
            time,
            frozen,
            options.debug_animation,
        );
        self.host.run_lerp_frame(
            ci,
            &mut entity.player.torso.base,
            state.torso_anim,
            speed_scale,
            time,
            frozen,
            options.debug_animation,
        );
        legs.old_frame = entity.player.legs.base.old_frame;
        legs.frame = entity.player.legs.base.frame;
        legs.back_lerp = entity.player.legs.base.back_lerp;
        torso.old_frame = entity.player.torso.base.old_frame;
        torso.frame = entity.player.torso.base.frame;
        torso.back_lerp = entity.player.torso.base.back_lerp;
    }

    fn sprite(
        &mut self,
        entity: &ClientEntity,
        shader: Option<SceneShader>,
        snapshot_client_num: i32,
        rendering_third_person: bool,
    ) {
        let mut sprite = create_sprite_entity();
        sprite.origin = add3(entity.lerp_origin, vec3(0.0, 0.0, 48.0));
        sprite.custom_shader = shader;
        sprite.radius = 10.0;
        sprite.render_flags = if entity.current_state.number == snapshot_client_num && !rendering_third_person {
            RF_THIRD_PERSON
        } else {
            0
        };
        sprite.shader_rgba = vec4(255.0, 255.0, 255.0, 255.0);
        self.host.add_entity(RefEntity::Sprite(sprite));
    }

    fn sprites(
        &mut self,
        entity: &ClientEntity,
        ci: &ClientInfo,
        options: &PlayerPresentationSettings,
        ctx: &PlayerSpriteContext,
    ) {
        let media = self.host.media();
        let choices: [(i32, Option<SceneShader>); 8] = [
            (0x2000, media.connection_shader.clone()),
            (0x1000, media.balloon_shader.clone()),
            (0x8000, media.medal_impressive.clone()),
            (8, media.medal_excellent.clone()),
            (64, media.medal_gauntlet.clone()),
            (0x10000, media.medal_defend.clone()),
            (0x20000, media.medal_assist.clone()),
            (0x800, media.medal_capture.clone()),
        ];
        for (flag, shader) in choices {
            if entity.current_state.e_flags & flag != 0 {
                self.sprite(entity, shader, ctx.snapshot_client_num, ctx.rendering_third_person);
                return;
            }
        }
        if entity.current_state.e_flags & PLAYER_DEAD == 0
            && ctx.snapshot_team == ci.team as i32
            && (options.game_type as i32) >= (GameType::GtTeam as i32)
            && options.draw_friend
        {
            self.sprite(
                entity,
                media.friend_shader.clone(),
                ctx.snapshot_client_num,
                ctx.rendering_third_person,
            );
        }
    }

    fn shadow(&mut self, entity: &ClientEntity, options: &PlayerPresentationSettings) -> PresentResult<ShadowOutcome> {
        if options.shadows == 0 || powered(&entity.current_state, Powerup::PwInvis) {
            return Ok(ShadowOutcome {
                visible: false,
                plane: 0.0,
            });
        }
        let trace = self.host.trace_world(
            entity.lerp_origin,
            add3(entity.lerp_origin, vec3(0.0, 0.0, -128.0)),
            vec3(-15.0, -15.0, 0.0),
            vec3(15.0, 15.0, 2.0),
            MASK_PLAYER_SOLID,
        );
        if trace.fraction == 1.0 || trace.solidity != TraceSolidity::Clear {
            return Ok(ShadowOutcome {
                visible: false,
                plane: 0.0,
            });
        }
        let plane = trace.end.z + 1.0;
        if options.shadows != 1 {
            return Ok(ShadowOutcome { visible: true, plane });
        }
        let TraceContact::Plane { plane } = trace.contact else {
            return Err(state_msg("Player shadow hit has no contact plane"));
        };
        let alpha = 1.0 - trace.fraction;
        let media = self.host.media();
        let polys = self.host.impact_mark(&ImpactMarkRequest {
            shader: media.shadow_mark_shader.clone(),
            origin: trace.end,
            direction: plane.normal,
            orientation: entity.player.legs.yaw_angle,
            color: vec4(alpha, alpha, alpha, 1.0),
            alpha_fade: false,
            radius: 24.0,
            temporary: true,
        });
        for poly in polys {
            self.host.add_poly(poly);
        }
        Ok(ShadowOutcome {
            visible: true,
            plane: trace.end.z + 1.0,
        })
    }

    fn splash(&mut self, entity: &ClientEntity, options: &PlayerPresentationSettings) {
        if options.shadows == 0 {
            return;
        }
        let end = add3(entity.lerp_origin, vec3(0.0, 0.0, -24.0));
        if self.host.point_contents(end) & MASK_WATER == 0 {
            return;
        }
        let start = add3(entity.lerp_origin, vec3(0.0, 0.0, 32.0));
        if self.host.point_contents(start) & (1 | MASK_WATER) != 0 {
            return;
        }
        let trace = self
            .host
            .trace_world(start, end, vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), MASK_WATER);
        if trace.fraction == 1.0 {
            return;
        }
        let vertex = |x: f32, y: f32, s: f32, t: f32| RefPolyVertex {
            position: add3(trace.end, vec3(x, y, 0.0)),
            tex_coord: [s, t],
            color: vec4(255.0, 255.0, 255.0, 255.0),
        };
        let media = self.host.media();
        self.host.add_poly(RefPoly {
            shader: media.wake_mark_shader.clone(),
            vertices: vec![
                vertex(-32.0, -32.0, 0.0, 0.0),
                vertex(-32.0, 32.0, 0.0, 1.0),
                vertex(32.0, 32.0, 1.0, 1.0),
                vertex(32.0, -32.0, 1.0, 0.0),
            ],
        });
    }

    fn haste_trail(&mut self, entity: &mut ClientEntity, time: i32) {
        if entity.trail_time > time {
            return;
        }
        let animation = entity.player.legs.base.animation_number & !ANIMATION_TOGGLE_BIT;
        if animation != PlayerAnim::LegsRun as i32 && animation != PlayerAnim::LegsBack as i32 {
            return;
        }
        entity.trail_time = entity.trail_time.wrapping_add(100);
        if entity.trail_time < time {
            entity.trail_time = time;
        }
        let media = self.host.media();
        self.host.smoke_puff(
            &SmokePuffOptions {
                origin: add3(entity.lerp_origin, vec3(0.0, 0.0, -16.0)),
                velocity: vec3(0.0, 0.0, 0.0),
                radius: 8.0,
                color: vec4(1.0, 1.0, 1.0, 1.0),
                duration: 500,
                start_time: time,
                fade_in_time: 0,
                flags: 0,
                shader: media.haste_puff_shader.clone(),
            },
            true,
        );
    }

    fn breath(
        &mut self,
        entity: &ClientEntity,
        head: &RefModelEntity,
        options: &PlayerPresentationSettings,
        media: &MissionPlayerMedia,
        clients: &mut [ClientInfo],
        ctx: &PlayerSpriteContext,
    ) -> PresentResult<()> {
        if !options.enable_breath
            || (entity.current_state.number == ctx.snapshot_client_num && !ctx.rendering_third_person)
            || entity.current_state.e_flags & PLAYER_DEAD != 0
        {
            return Ok(());
        }
        let number = entity.current_state.number;
        if number < 0 {
            return Err(range_msg(format!("Player index {number} outside clients")));
        }
        let ci = at_mut(clients, number as usize, "Player index")?;
        if self.host.point_contents(head.origin) & MASK_WATER != 0 || ci.breath_puff_time > ctx.time {
            return Ok(());
        }
        self.host.smoke_puff(
            &SmokePuffOptions {
                origin: add3(add3(head.origin, scale3(head.axis[0], 8.0)), scale3(head.axis[2], -4.0)),
                velocity: vec3(0.0, 0.0, 8.0),
                radius: 16.0,
                color: vec4(1.0, 1.0, 1.0, 0.66),
                duration: 1500,
                start_time: ctx.time,
                fade_in_time: ctx.time.wrapping_add(400),
                flags: 1,
                shader: media.shotgun_smoke_puff_shader.clone(),
            },
            false,
        );
        let ci = at_mut(clients, number as usize, "Player index")?;
        ci.breath_puff_time = ctx.time.wrapping_add(2000);
        Ok(())
    }

    fn dust(
        &mut self,
        entity: &mut ClientEntity,
        options: &PlayerPresentationSettings,
        media: &MissionPlayerMedia,
        time: i32,
    ) {
        if !options.enable_dust || entity.dust_trail_time > time {
            return;
        }
        let animation = entity.player.legs.base.animation_number & !ANIMATION_TOGGLE_BIT;
        if animation != PlayerAnim::LegsLandb as i32 && animation != PlayerAnim::LegsLand as i32 {
            return;
        }
        entity.dust_trail_time = entity.dust_trail_time.wrapping_add(40);
        if entity.dust_trail_time < time {
            entity.dust_trail_time = time;
        }
        let origin = entity.current_state.pos.base;
        let trace = self.host.trace_skip(
            origin,
            add3(origin, vec3(0.0, 0.0, -64.0)),
            vec3(0.0, 0.0, 0.0),
            vec3(0.0, 0.0, 0.0),
            entity.current_state.number,
            MASK_PLAYER_SOLID,
        );
        if trace.surface_flags & 0x40000 == 0 {
            return;
        }
        self.host.smoke_puff(
            &SmokePuffOptions {
                origin: add3(origin, vec3(0.0, 0.0, -16.0)),
                velocity: vec3(0.0, 0.0, -30.0),
                radius: 24.0,
                color: vec4(0.8, 0.8, 0.7, 0.33),
                duration: 500,
                start_time: time,
                fade_in_time: 0,
                flags: 0,
                shader: media.dust_puff_shader.clone(),
            },
            false,
        );
    }

    fn trail_item(&mut self, entity: &ClientEntity, model: SceneModel) {
        let axis = angles_to_axis(vec3(0.0, entity.lerp_angles.y, 0.0));
        let mut item = create_model_entity_with(model);
        item.origin = add3(add3(entity.lerp_origin, scale3(axis[0], -16.0)), vec3(0.0, 0.0, 16.0));
        item.axis = angles_to_axis(vec3(0.0, entity.lerp_angles.y + 90.0, 0.0));
        self.host.add_entity(RefEntity::Model(item));
    }

    #[allow(clippy::too_many_arguments)]
    fn flag(
        &mut self,
        entity: &mut ClientEntity,
        skin: Option<SceneSkin>,
        torso: &RefModelEntity,
        options: &PlayerPresentationSettings,
        ci: &ClientInfo,
        time: i32,
        frame_time: i32,
    ) -> PresentResult<()> {
        let media = self.host.media();
        let mut pole = create_model_entity_with(media.flag_pole_model.clone());
        let mut flag = create_model_entity_with(media.flag_flap_model.clone());
        pole.lighting_origin = torso.lighting_origin;
        pole.shadow_plane = torso.shadow_plane;
        pole.render_flags = torso.render_flags;
        let pole_model = pole.model.clone();
        self.host.position_on_tag(&mut pole, torso, &pole_model, "tag_flag")?;
        self.host.add_entity(RefEntity::Model(pole.clone()));
        flag.custom_skin = skin;
        flag.lighting_origin = torso.lighting_origin;
        flag.shadow_plane = torso.shadow_plane;
        flag.render_flags = torso.render_flags;
        let legs = entity.current_state.legs_anim & !ANIMATION_TOGGLE_BIT;
        let idle = legs == PlayerAnim::LegsIdle as i32 || legs == PlayerAnim::LegsIdlecr as i32;
        let walk = legs == PlayerAnim::LegsWalk as i32 || legs == PlayerAnim::LegsWalkcr as i32;
        if !idle {
            let direction = normalize3(add3(entity.current_state.pos.delta, vec3(0.0, 0.0, 100.0)));
            if dot3(pole.axis[2], direction).abs() < 0.9 {
                let angle = dot3(pole.axis[0], direction).clamp(-1.0, 1.0).acos();
                let degrees = angle * 180.0 / std::f32::consts::PI;
                let mut yaw = if dot3(pole.axis[1], direction) < 0.0 {
                    360.0 - degrees
                } else {
                    degrees
                };
                if yaw < 0.0 {
                    yaw += 360.0;
                }
                if yaw > 360.0 {
                    yaw -= 360.0;
                }
                let (angle, swinging) = self.host.swing_angles(
                    yaw,
                    25.0,
                    90.0,
                    0.15,
                    frame_time,
                    entity.player.flag.yaw_angle,
                    entity.player.flag.yawing,
                );
                entity.player.flag.yaw_angle = angle;
                entity.player.flag.yawing = swinging;
            }
        }
        let frozen = options.animation_speed == 0.0;
        let new_animation = if idle || walk {
            PlayerAnim::FlagStand as i32
        } else {
            PlayerAnim::FlagRun as i32
        };
        self.host.run_lerp_frame(
            ci,
            &mut entity.player.flag.base,
            new_animation,
            1.0,
            time,
            frozen,
            options.debug_animation,
        );
        flag.old_frame = entity.player.flag.base.old_frame;
        flag.frame = entity.player.flag.base.frame;
        flag.back_lerp = entity.player.flag.base.back_lerp;
        flag.axis = angles_to_axis(vec3(0.0, entity.player.flag.yaw_angle, 0.0));
        let pole_model = pole.model.clone();
        self.host
            .position_rotated_on_tag(&mut flag, &pole, &pole_model, "tag_flag")?;
        self.host.add_entity(RefEntity::Model(flag));
        Ok(())
    }

    fn powerups(
        &mut self,
        entity: &mut ClientEntity,
        torso: &RefModelEntity,
        ci: &ClientInfo,
        options: &PlayerPresentationSettings,
        time: i32,
        frame_time: i32,
    ) -> PresentResult<()> {
        let state = entity.current_state.clone();
        if state.powerups == 0 {
            return Ok(());
        }
        let media = self.host.media();
        if powered(&state, Powerup::PwQuad) {
            let radius = 200.0 + (self.host.random_int() & 31) as f32;
            self.host.add_light(DynamicLight {
                origin: entity.lerp_origin,
                radius,
                color: vec3(0.2, 0.2, 1.0),
                additive: false,
            });
        }
        if powered(&state, Powerup::PwFlight) {
            self.host.add_looping_sound(
                state.number,
                entity.lerp_origin,
                vec3(0.0, 0.0, 0.0),
                media.flight_sound,
            );
        }
        let flags: [(Powerup, SceneModel, Option<SceneSkin>, Vec3); 3] = [
            (
                Powerup::PwRedflag,
                media.red_flag_model.clone(),
                media.red_flag_flap_skin.clone(),
                vec3(1.0, 0.2, 0.2),
            ),
            (
                Powerup::PwBlueflag,
                media.blue_flag_model.clone(),
                media.blue_flag_flap_skin.clone(),
                vec3(0.2, 0.2, 1.0),
            ),
            (
                Powerup::PwNeutralflag,
                media.neutral_flag_model.clone(),
                media.neutral_flag_flap_skin.clone(),
                vec3(1.0, 1.0, 1.0),
            ),
        ];
        for (powerup, model, skin, color) in flags {
            if powered(&state, powerup) {
                if ci.new_anims {
                    let torso = torso.clone();
                    self.flag(entity, skin, &torso, options, ci, time, frame_time)?;
                } else {
                    self.trail_item(entity, model);
                }
                let radius = 200.0 + (self.host.random_int() & 31) as f32;
                self.host.add_light(DynamicLight {
                    origin: entity.lerp_origin,
                    radius,
                    color,
                    additive: false,
                });
            }
        }
        if powered(&state, Powerup::PwHaste) {
            self.haste_trail(entity, time);
        }
        Ok(())
    }

    fn tokens(
        &mut self,
        entity: &mut ClientEntity,
        ci: &ClientInfo,
        render_flags: i32,
        media: &MissionPlayerMedia,
        skull_trails: &mut [SkullTrail; MAX_CLIENTS],
        time: i32,
    ) -> PresentResult<()> {
        let tokens = entity.current_state.generic1.min(10);
        let number = entity.current_state.number;
        let Some(trail) = skull_trails.get_mut(number as usize) else {
            // Corpses have non-client entity numbers. Omit source's out-of-bounds zero write.
            if tokens == 0 {
                return Ok(());
            }
            return Err(range_msg(format!("No skull trail for entity {number}")));
        };
        if tokens == 0 {
            trail.num_positions = 0;
            return Ok(());
        }
        for _ in 0..tokens - trail.num_positions {
            for j in (1..=trail.num_positions as usize).rev() {
                trail.positions[j] = trail.positions[j - 1];
            }
            trail.positions[0] = entity.lerp_origin;
        }
        trail.num_positions = tokens;
        let mut origin = entity.lerp_origin;
        for i in 0..trail.num_positions as usize {
            let delta = sub3(trail.positions[i], origin);
            if length3(delta) > 30.0 {
                trail.positions[i] = add3(origin, scale3(normalize3(delta), 30.0));
            }
            origin = trail.positions[i];
        }
        let mut skull = create_model_entity_with(if ci.team == Team::TeamBlue {
            media.red_cube_model.clone()
        } else {
            media.blue_cube_model.clone()
        });
        skull.render_flags = render_flags;
        let mut origin = entity.lerp_origin;
        for i in 0..trail.num_positions as usize {
            let position = trail.positions[i];
            let delta = sub3(origin, position);
            let forward = normalize3(vec3(delta.x, delta.y, 0.0));
            let up = vec3(0.0, 0.0, 1.0);
            skull.axis = [forward, cross3(forward, up), up];
            let angle = sphere_angle(time.wrapping_add(500 * 10 - 500 * i as i32), 16);
            skull.origin = add3(position, vec3(0.0, 0.0, angle.sin() * 10.0));
            self.host.add_entity(RefEntity::Model(skull.clone()));
            origin = position;
        }
        Ok(())
    }

    fn kamikaze_pair(&mut self, skull: &mut RefModelEntity, media: &MissionPlayerMedia, flip_trail: bool) {
        skull.model = media.kamikaze_head_model.clone();
        self.host.add_entity(RefEntity::Model(skull.clone()));
        if flip_trail {
            skull.axis[1] = scale3(skull.axis[1], -1.0);
        }
        skull.model = media.kamikaze_head_trail.clone();
        self.host.add_entity(RefEntity::Model(skull.clone()));
    }

    fn kamikaze(&mut self, entity: &ClientEntity, torso: &RefModelEntity, media: &MissionPlayerMedia, time: i32) {
        let mut skull = create_model_entity();
        skull.lighting_origin = entity.lerp_origin;
        skull.shadow_plane = torso.shadow_plane;
        skull.render_flags = torso.render_flags;
        if entity.current_state.e_flags & PLAYER_DEAD != 0 {
            let mut angle = sphere_angle(time, 7);
            if angle > std::f32::consts::PI * 2.0 {
                angle -= std::f32::consts::PI * 2.0;
            }
            let direction = vec3(
                angle.sin() * 20.0,
                angle.cos() * 20.0,
                15.0 + sphere_angle(time, 4).sin() * 8.0,
            );
            skull.origin = add3(torso.origin, direction);
            let side = normalize3(vec3(direction.x, direction.y, 0.0));
            let up = vec3(0.0, 0.0, 1.0);
            skull.axis = [cross3(side, up), side, up];
            let media = media.clone();
            self.kamikaze_pair(&mut skull, &media, false);
            return;
        }
        let angle = sphere_angle(time, 4);
        let direction = vec3(angle.cos() * 20.0, angle.sin() * 20.0, angle.cos() * 20.0);
        skull.origin = add3(torso.origin, direction);
        let mut yaw = angle * 180.0 / std::f32::consts::PI + 90.0;
        if yaw > 360.0 {
            yaw -= 360.0;
        }
        skull.axis = angles_to_axis(vec3(angle.sin() * 30.0, yaw, 0.0));
        let media = media.clone();
        self.kamikaze_pair(&mut skull, &media, true);
        let mut angle = sphere_angle(time, 4) + std::f32::consts::PI;
        if angle > std::f32::consts::PI * 2.0 {
            angle -= std::f32::consts::PI * 2.0;
        }
        let direction = vec3(angle.sin() * 20.0, angle.cos() * 20.0, angle.cos() * 20.0);
        skull.origin = add3(torso.origin, direction);
        let mut yaw = 360.0 - angle * 180.0 / std::f32::consts::PI;
        if yaw > 360.0 {
            yaw -= 360.0;
        }
        skull.axis = angles_to_axis(vec3((angle - 0.5 * std::f32::consts::PI).cos() * 30.0, yaw, 0.0));
        self.kamikaze_pair(&mut skull, &media, false);
        let mut angle = sphere_angle(time, 3) + 0.5 * std::f32::consts::PI;
        if angle > std::f32::consts::PI * 2.0 {
            angle -= std::f32::consts::PI * 2.0;
        }
        let direction = vec3(angle.sin() * 20.0, angle.cos() * 20.0, 0.0);
        skull.origin = add3(torso.origin, direction);
        let side = normalize3(vec3(direction.x, direction.y, 0.0));
        let up = vec3(0.0, 0.0, 1.0);
        skull.axis = [cross3(side, up), side, up];
        self.kamikaze_pair(&mut skull, &media, false);
    }

    fn mission_powerups(
        &mut self,
        entity: &ClientEntity,
        ci_index: usize,
        torso: &RefModelEntity,
        media: &MissionPlayerMedia,
        clients: &mut [ClientInfo],
        time: i32,
    ) -> PresentResult<()> {
        if entity.current_state.e_flags & PLAYER_KAMIKAZE != 0 {
            self.kamikaze(entity, torso, media, time);
        }
        let attachments: [(Powerup, SceneModel); 4] = [
            (Powerup::PwGuard, media.guard_powerup_model.clone()),
            (Powerup::PwScout, media.scout_powerup_model.clone()),
            (Powerup::PwDoubler, media.doubler_powerup_model.clone()),
            (Powerup::PwAmmoregen, media.ammo_regen_powerup_model.clone()),
        ];
        for (powerup, model) in attachments {
            if powered(&entity.current_state, powerup) {
                let mut attachment = torso.clone();
                attachment.model = model;
                attachment.frame = 0;
                attachment.old_frame = 0;
                attachment.custom_skin = None;
                self.host.add_entity(RefEntity::Model(attachment));
            }
        }
        let invulnerable = powered(&entity.current_state, Powerup::PwInvulnerability);
        {
            let ci = at_mut(clients, ci_index, "Player index")?;
            if invulnerable {
                if ci.invulnerability_start_time == 0 {
                    ci.invulnerability_start_time = time;
                }
                ci.invulnerability_stop_time = time;
            } else {
                ci.invulnerability_start_time = 0;
            }
        }
        let ci = at(clients, ci_index, "Player index")?;
        let since_start = time.wrapping_sub(ci.invulnerability_start_time);
        let since_stop = time.wrapping_sub(ci.invulnerability_stop_time);
        if invulnerable || since_stop < 250 {
            let mut shell = torso.clone();
            shell.model = media.invulnerability_powerup_model.clone();
            shell.custom_skin = None;
            shell.render_flags &= !RF_THIRD_PERSON;
            shell.origin = entity.lerp_origin;
            let scale = if since_start < 250 {
                since_start as f32 / 250.0
            } else if since_stop < 250 {
                (250 - since_stop) as f32 / 250.0
            } else {
                1.0
            };
            shell.axis = [vec3(scale, 0.0, 0.0), vec3(0.0, scale, 0.0), vec3(0.0, 0.0, scale)];
            self.host.add_entity(RefEntity::Model(shell));
        }
        let ci = at(clients, ci_index, "Player index")?;
        let elapsed = time.wrapping_sub(ci.medkit_usage_time);
        if ci.medkit_usage_time != 0 && elapsed < 500 {
            let mut medkit = torso.clone();
            medkit.model = media.medkit_usage_model.clone();
            medkit.custom_skin = None;
            medkit.render_flags &= !RF_THIRD_PERSON;
            medkit.axis = angles_to_axis(vec3(0.0, 0.0, 0.0));
            medkit.origin = add3(
                entity.lerp_origin,
                vec3(0.0, 0.0, -24.0 + elapsed as f32 * 80.0 / 500.0),
            );
            let c = if elapsed > 400 {
                (255.0 - (elapsed.wrapping_sub(1000)) as f32 * 255.0 / 100.0) as i32 & 255
            } else {
                255
            };
            medkit.shader_rgba = vec4(c as f32, c as f32, c as f32, c as f32);
            self.host.add_entity(RefEntity::Model(medkit));
        }
        Ok(())
    }

    /// Present a player entity.
    pub fn player(&mut self, ctx: &mut PlayerRenderContext<'_>, entity: &mut ClientEntity) -> PresentResult<()> {
        let client_num = entity.current_state.client_num;
        if client_num < 0 || client_num >= MAX_CLIENTS as i32 {
            return Err(drop_msg("Bad clientNum on player entity"));
        }
        let ci_index = client_num as usize;
        let PlayerRenderContext {
            time,
            frame_time,
            rendering_third_person,
            snapshot_client_num,
            snapshot_team,
            clients,
            skull_trails,
        } = &mut *ctx;
        let (time, frame_time, rendering_third_person, snapshot_client_num, snapshot_team) = (
            *time,
            *frame_time,
            *rendering_third_person,
            *snapshot_client_num,
            *snapshot_team,
        );
        if !at(clients, ci_index, "Player index")?.info_valid {
            return Ok(());
        }
        let options = self.host.settings();
        let body_visible = !self.host.body_hidden(entity.current_state.number);
        let mut render_flags = 0;
        if entity.current_state.number == snapshot_client_num {
            if !rendering_third_person {
                render_flags = RF_THIRD_PERSON;
            } else if options.camera_mode {
                return Ok(());
            }
        }
        let mut legs = create_model_entity();
        let mut torso = create_model_entity();
        let mut head = create_model_entity();
        let pose = {
            let ci = at(clients, ci_index, "Player index")?;
            self.host.calculate_pose(
                &entity.player,
                &entity.current_state,
                ci,
                entity.lerp_angles,
                time,
                frame_time,
                options.swing_speed,
            )
        };
        legs.axis = pose.legs;
        torso.axis = pose.torso;
        head.axis = pose.head;
        {
            let ci = at(clients, ci_index, "Player index")?.clone();
            self.animate(entity, &ci, &mut legs, &mut torso, &options, time);
        }
        let sprite_ctx = PlayerSpriteContext {
            time,
            rendering_third_person,
            snapshot_client_num,
            snapshot_team,
        };
        if body_visible {
            let ci = at(clients, ci_index, "Player index")?.clone();
            self.sprites(entity, &ci, &options, &sprite_ctx);
        }
        let shadow = if body_visible {
            self.shadow(entity, &options)?
        } else {
            ShadowOutcome {
                visible: false,
                plane: 0.0,
            }
        };
        if body_visible {
            self.splash(entity, &options);
        }
        if options.shadows == 3 && shadow.visible {
            render_flags |= RF_SHADOW_PLANE;
        }
        render_flags |= RF_LIGHTING_ORIGIN;
        let mission_media = self.host.mission_media();
        if self.host.product() == Product::Missionpack && options.game_type == GameType::GtHarvester {
            let media = mission_media
                .as_ref()
                .ok_or_else(|| state_msg("Mission player media requires missionpack"))?;
            let ci = at(clients, ci_index, "Player index")?.clone();
            self.tokens(entity, &ci, render_flags, media, skull_trails, time)?;
        }
        let ci = at(clients, ci_index, "Player index")?.clone();
        legs.model = ci.legs_model.clone();
        legs.custom_skin = ci.legs_skin.clone();
        legs.origin = entity.lerp_origin;
        legs.lighting_origin = entity.lerp_origin;
        legs.shadow_plane = shadow.plane;
        legs.render_flags = render_flags;
        legs.old_origin = legs.origin;
        let current = entity.current_state.clone();
        self.add_ref_entity_with_powerups(&mut legs, &current, ci.team, Some(QvmBodyPart::Lower), time);
        if legs.model.is_default() || ci.torso_model.is_default() {
            return Ok(());
        }
        torso.model = ci.torso_model.clone();
        torso.custom_skin = ci.torso_skin.clone();
        torso.lighting_origin = entity.lerp_origin;
        let legs_model = ci.legs_model.clone();
        self.host
            .position_rotated_on_tag(&mut torso, &legs, &legs_model, "tag_torso")?;
        torso.shadow_plane = shadow.plane;
        torso.render_flags = render_flags;
        self.add_ref_entity_with_powerups(&mut torso, &current, ci.team, Some(QvmBodyPart::Upper), time);
        if self.host.product() == Product::Missionpack {
            let media = mission_media
                .as_ref()
                .ok_or_else(|| state_msg("Mission player media requires missionpack"))?;
            self.mission_powerups(entity, ci_index, &torso, media, clients, time)?;
        }
        if ci.head_model.is_default() {
            return Ok(());
        }
        head.model = ci.head_model.clone();
        head.custom_skin = ci.head_skin.clone();
        head.lighting_origin = entity.lerp_origin;
        let torso_model = ci.torso_model.clone();
        self.host
            .position_rotated_on_tag(&mut head, &torso, &torso_model, "tag_head")?;
        head.shadow_plane = shadow.plane;
        head.render_flags = render_flags;
        self.add_ref_entity_with_powerups(&mut head, &current, ci.team, Some(QvmBodyPart::Head), time);
        if body_visible && self.host.product() == Product::Missionpack {
            let media = mission_media
                .as_ref()
                .ok_or_else(|| state_msg("Mission player media requires missionpack"))?;
            self.breath(entity, &head, &options, media, clients, &sprite_ctx)?;
            self.dust(entity, &options, media, time);
        }
        self.host.add_player_weapon(&torso, None, entity, ci.team);
        self.powerups(entity, &torso, &ci, &options, time, frame_time)?;
        Ok(())
    }
}

/// Snapshot inputs shared by sprite helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlayerSpriteContext {
    time: i32,
    rendering_third_person: bool,
    snapshot_client_num: i32,
    snapshot_team: i32,
}

/// Client model file lookup (`ClientFile`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientFile {
    /// File found.
    Found {
        /// File path.
        path: String,
    },
    /// File missing; path is the last candidate.
    Missing {
        /// Last candidate path.
        path: String,
    },
}

/// Client info store (`ClientInfoStore`).
///
/// Canonical slots live in `ClientGameStaticState.client_info`; every method
/// takes them explicitly. Donor `async` load ordering is synchronous here, so
/// superseded-load checks fold into the request/lifecycle counters.
pub struct ClientInfoStore<H> {
    /// Host services.
    pub host: H,
    requests: [u32; MAX_CLIENTS],
    lifecycle: u64,
}

impl<H: ClientInfoHost> ClientInfoStore<H> {
    /// New store.
    #[must_use]
    pub fn new(host: H) -> Self {
        Self {
            host,
            requests: [0; MAX_CLIENTS],
            lifecycle: 0,
        }
    }

    /// Client info slot.
    pub fn client_info<'a>(&self, slots: &'a [ClientInfo], index: i32) -> PresentResult<&'a ClientInfo> {
        if index < 0 {
            return Err(range_msg(format!("Player index {index} outside clients")));
        }
        at(slots, index as usize, "Player index")
    }

    /// Invalidate pending loads and clear every slot.
    pub fn reset(&mut self, slots: &mut [ClientInfo]) {
        self.lifecycle = self.lifecycle.wrapping_add(1);
        for slot in slots.iter_mut() {
            *slot = ClientInfo::default();
        }
    }

    /// Custom sound for a client.
    pub fn custom_sound(&mut self, slots: &[ClientInfo], index: i32, name: &str) -> PresentResult<Option<PcmSound>> {
        if !name.starts_with('*') {
            return Ok(self.host.sound(name));
        }
        let slot_index = if index < 0 || index >= MAX_CLIENTS as i32 {
            0
        } else {
            index
        };
        let slot = self.client_info(slots, slot_index)?;
        let Some(sound_index) = CUSTOM_SOUND_NAMES.iter().position(|value| *value == name) else {
            return Err(drop_msg(format!("Unknown custom sound: {name}")));
        };
        Ok(*at(&slot.sounds, sound_index, "Player index")?)
    }

    fn team_folder(&self, ci: &ClientInfo, settings: &ClientInfoSettings) -> String {
        if (settings.game_type as i32) >= (GameType::GtTeam as i32) {
            if ci.team == Team::TeamBlue {
                "blue".to_string()
            } else {
                "red".to_string()
            }
        } else {
            "default".to_string()
        }
    }

    fn filename(&mut self, length: usize, format: &str, args: &[GameFormatArgument]) -> PresentResult<String> {
        let filename = game_format_bounded(format, args, usize::MAX);
        if filename.len() >= length {
            let message = game_format_bounded(
                "Com_sprintf: overflow of %i in %i\n",
                &[
                    GameFormatArgument::from(filename.len() as i32),
                    GameFormatArgument::from(length as i32),
                ],
                1024,
            );
            self.host.print(&message);
        }
        Ok(filename.chars().take(length - 1).collect())
    }

    fn exists(&mut self, path: &str) -> PresentResult<bool> {
        if !self.host.has_asset(path) {
            return Ok(false);
        }
        let bytes = self.host.read_asset(path)?;
        Ok(!bytes.is_empty())
    }

    #[allow(clippy::too_many_arguments)]
    fn find_model(
        &mut self,
        ci: &ClientInfo,
        settings: &ClientInfoSettings,
        team_name: &str,
        model: &str,
        skin: &str,
        base: &str,
        ext: &str,
    ) -> PresentResult<ClientFile> {
        let team = self.team_folder(ci, settings);
        let mut filename = String::new();
        let folders = ["", "characters/"];
        let prefixes: Vec<&str> = if team_name.is_empty() {
            vec![""]
        } else {
            vec![team_name, ""]
        };
        for folder in folders {
            for prefix in &prefixes {
                filename = self.filename(
                    64,
                    "models/players/%s%s/%s%s_%s_%s.%s",
                    &[
                        GameFormatArgument::from(folder),
                        GameFormatArgument::from(model),
                        GameFormatArgument::from(*prefix),
                        GameFormatArgument::from(base),
                        GameFormatArgument::from(skin),
                        GameFormatArgument::from(team.as_str()),
                        GameFormatArgument::from(ext),
                    ],
                )?;
                if self.exists(&filename)? {
                    return Ok(ClientFile::Found { path: filename });
                }
                let patch = if (settings.game_type as i32) >= (GameType::GtTeam as i32) {
                    team.as_str()
                } else {
                    skin
                };
                filename = self.filename(
                    64,
                    "models/players/%s%s/%s%s_%s.%s",
                    &[
                        GameFormatArgument::from(folder),
                        GameFormatArgument::from(model),
                        GameFormatArgument::from(*prefix),
                        GameFormatArgument::from(base),
                        GameFormatArgument::from(patch),
                        GameFormatArgument::from(ext),
                    ],
                )?;
                if self.exists(&filename)? {
                    return Ok(ClientFile::Found { path: filename });
                }
            }
        }
        Ok(ClientFile::Missing { path: filename })
    }

    #[allow(clippy::too_many_arguments)]
    fn find_head(
        &mut self,
        ci: &ClientInfo,
        settings: &ClientInfoSettings,
        team_name: &str,
        model: &str,
        skin: &str,
        base: &str,
        ext: &str,
        length: usize,
    ) -> PresentResult<ClientFile> {
        let team = self.team_folder(ci, settings);
        let name = model.strip_prefix('*').unwrap_or(model);
        let mut filename = String::new();
        let folders: Vec<&str> = if model.starts_with('*') {
            vec!["heads/"]
        } else {
            vec!["", "heads/"]
        };
        let prefixes: Vec<&str> = if team_name.is_empty() {
            vec![""]
        } else {
            vec![team_name, ""]
        };
        for folder in folders {
            for prefix in &prefixes {
                filename = self.filename(
                    length,
                    "models/players/%s%s/%s/%s%s_%s.%s",
                    &[
                        GameFormatArgument::from(folder),
                        GameFormatArgument::from(name),
                        GameFormatArgument::from(skin),
                        GameFormatArgument::from(*prefix),
                        GameFormatArgument::from(base),
                        GameFormatArgument::from(team.as_str()),
                        GameFormatArgument::from(ext),
                    ],
                )?;
                if self.exists(&filename)? {
                    return Ok(ClientFile::Found { path: filename });
                }
                let patch = if (settings.game_type as i32) >= (GameType::GtTeam as i32) {
                    team.as_str()
                } else {
                    skin
                };
                filename = self.filename(
                    length,
                    "models/players/%s%s/%s%s_%s.%s",
                    &[
                        GameFormatArgument::from(folder),
                        GameFormatArgument::from(name),
                        GameFormatArgument::from(*prefix),
                        GameFormatArgument::from(base),
                        GameFormatArgument::from(patch),
                        GameFormatArgument::from(ext),
                    ],
                )?;
                if self.exists(&filename)? {
                    return Ok(ClientFile::Found { path: filename });
                }
            }
        }
        Ok(ClientFile::Missing { path: filename })
    }

    #[allow(clippy::too_many_arguments)]
    fn skin(
        &mut self,
        ci: &mut ClientInfo,
        settings: &ClientInfoSettings,
        team: &str,
        model: &str,
        skin: &str,
        head: &str,
        head_skin: &str,
    ) -> PresentResult<bool> {
        let lower = self.find_model(ci, settings, team, model, skin, "lower", "skin")?;
        if let ClientFile::Found { path } = &lower {
            ci.legs_skin = self.host.register_skin(path)?;
        }
        if ci.legs_skin.is_none() {
            let path = match &lower {
                ClientFile::Found { path } | ClientFile::Missing { path } => path.clone(),
            };
            self.host.print(&format!("Leg skin load failure: {path}\n"));
        }
        let upper = self.find_model(ci, settings, team, model, skin, "upper", "skin")?;
        if let ClientFile::Found { path } = &upper {
            ci.torso_skin = self.host.register_skin(path)?;
        }
        if ci.torso_skin.is_none() {
            let path = match &upper {
                ClientFile::Found { path } | ClientFile::Missing { path } => path.clone(),
            };
            self.host.print(&format!("Torso skin load failure: {path}\n"));
        }
        let face = self.find_head(ci, settings, team, head, head_skin, "head", "skin", 64)?;
        if let ClientFile::Found { path } = &face {
            ci.head_skin = self.host.register_skin(path)?;
        }
        if ci.head_skin.is_none() {
            let path = match &face {
                ClientFile::Found { path } | ClientFile::Missing { path } => path.clone(),
            };
            self.host.print(&format!("Head skin load failure: {path}\n"));
        }
        Ok(ci.legs_skin.is_some() && ci.torso_skin.is_some() && ci.head_skin.is_some())
    }

    fn animation(&mut self, ci: &mut ClientInfo, path: &str) -> PresentResult<bool> {
        if !self.host.has_asset(path) {
            return Ok(false);
        }
        let bytes = self.host.read_asset(path)?;
        if bytes.is_empty() {
            return Ok(false);
        }
        let text: String = bytes.iter().map(|b| char::from(*b)).collect();
        self.host.parse_animation_config(ci, &text, path)
    }

    #[allow(clippy::too_many_arguments)]
    fn model(
        &mut self,
        ci: &mut ClientInfo,
        settings: &ClientInfoSettings,
        model: &str,
        skin: &str,
        head_model: &str,
        head_skin: &str,
        team: &str,
    ) -> PresentResult<bool> {
        let head = if head_model.is_empty() { model } else { head_model };
        let mut path = self.filename(128, "models/players/%s/lower.md3", &[GameFormatArgument::from(model)])?;
        let mut handle = self.host.register_model(&path)?;
        ci.legs_model = handle.clone();
        if ci.legs_model.is_default() {
            path = self.filename(
                128,
                "models/players/characters/%s/lower.md3",
                &[GameFormatArgument::from(model)],
            )?;
            handle = self.host.register_model(&path)?;
            ci.legs_model = handle.clone();
        }
        if ci.legs_model.is_default() {
            let message = game_format_bounded(
                "Failed to load model file %s\n",
                &[GameFormatArgument::from(path.as_str())],
                1024,
            );
            self.host.print(&message);
            return Ok(false);
        }
        path = self.filename(128, "models/players/%s/upper.md3", &[GameFormatArgument::from(model)])?;
        handle = self.host.register_model(&path)?;
        ci.torso_model = handle.clone();
        if ci.torso_model.is_default() {
            path = self.filename(
                128,
                "models/players/characters/%s/upper.md3",
                &[GameFormatArgument::from(model)],
            )?;
            handle = self.host.register_model(&path)?;
            ci.torso_model = handle.clone();
        }
        if ci.torso_model.is_default() {
            let message = game_format_bounded(
                "Failed to load model file %s\n",
                &[GameFormatArgument::from(path.as_str())],
                1024,
            );
            self.host.print(&message);
            return Ok(false);
        }
        let star = head.starts_with('*');
        path = if star {
            let bare = head_model.get(1..).unwrap_or_default();
            self.filename(
                128,
                "models/players/heads/%s/%s.md3",
                &[GameFormatArgument::from(bare), GameFormatArgument::from(bare)],
            )?
        } else {
            self.filename(128, "models/players/%s/head.md3", &[GameFormatArgument::from(head)])?
        };
        handle = self.host.register_model(&path)?;
        ci.head_model = handle.clone();
        if ci.head_model.is_default() && !star {
            path = self.filename(
                128,
                "models/players/heads/%s/%s.md3",
                &[
                    GameFormatArgument::from(head_model),
                    GameFormatArgument::from(head_model),
                ],
            )?;
            handle = self.host.register_model(&path)?;
            ci.head_model = handle.clone();
        }
        if ci.head_model.is_default() {
            let message = game_format_bounded(
                "Failed to load model file %s\n",
                &[GameFormatArgument::from(path.as_str())],
                1024,
            );
            self.host.print(&message);
            return Ok(false);
        }
        let mut loaded_skin = self.skin(ci, settings, team, model, skin, head, head_skin)?;
        if !loaded_skin {
            if team.is_empty() {
                self.host.print(&format!(
                    "Failed to load skin file: {model} : {skin}, {head} : {head_skin}\n"
                ));
                return Ok(false);
            }
            self.host.print(&format!(
                "Failed to load skin file: {team} : {model} : {skin}, {head} : {head_skin}\n"
            ));
            let fallback_team = self.filename(
                128,
                "%s/",
                &[GameFormatArgument::from(if ci.team == Team::TeamBlue {
                    "Pagans"
                } else {
                    "Stroggs"
                })],
            )?;
            loaded_skin = self.skin(ci, settings, &fallback_team, model, skin, head, head_skin)?;
            if !loaded_skin {
                self.host.print(&format!(
                    "Failed to load skin file: {fallback_team} : {model} : {skin}, {head} : {head_skin}\n"
                ));
                return Ok(false);
            }
        }
        path = self.filename(
            128,
            "models/players/%s/animation.cfg",
            &[GameFormatArgument::from(model)],
        )?;
        let mut animated = self.animation(ci, &path)?;
        if !animated {
            path = self.filename(
                128,
                "models/players/characters/%s/animation.cfg",
                &[GameFormatArgument::from(model)],
            )?;
            animated = self.animation(ci, &path)?;
            if !animated {
                let message = game_format_bounded(
                    "Failed to load animation file %s\n",
                    &[GameFormatArgument::from(path.as_str())],
                    1024,
                );
                self.host.print(&message);
                return Ok(false);
            }
        }
        let mut icon = self.find_head(ci, settings, team, head, head_skin, "icon", "skin", 128)?;
        if matches!(icon, ClientFile::Missing { .. }) {
            icon = self.find_head(ci, settings, team, head, head_skin, "icon", "tga", 128)?;
        }
        if let ClientFile::Found { path } = icon {
            ci.model_icon = self.host.register_shader_no_mip(&path)?;
        }
        Ok(ci.model_icon.is_some())
    }

    fn load(&mut self, ci: &mut ClientInfo, settings: &ClientInfoSettings) -> PresentResult<()> {
        let product = self.host.product();
        let team_model = if product == Product::Missionpack {
            "james"
        } else {
            "sarge"
        };
        let team_head = if product == Product::Missionpack {
            "*james"
        } else {
            "sarge"
        };
        let mut team = if product == Product::Missionpack && (settings.game_type as i32) >= (GameType::GtTeam as i32) {
            qpath_truncate(if ci.team == Team::TeamBlue {
                &settings.blue_team_name
            } else {
                &settings.red_team_name
            })
        } else {
            String::new()
        };
        if !team.is_empty() {
            team.push('/');
        }
        let loaded = self.model(
            ci,
            settings,
            &ci.model_name.clone(),
            &ci.skin_name.clone(),
            &ci.head_model_name.clone(),
            &ci.head_skin_name.clone(),
            &team.clone(),
        )?;
        if !loaded {
            if settings.build_script {
                return Err(drop_msg(format!(
                    "CG_RegisterClientModelname( {}, {}, {}, {} {} ) failed",
                    ci.model_name, ci.skin_name, ci.head_model_name, ci.head_skin_name, team
                )));
            }
            if (settings.game_type as i32) >= (GameType::GtTeam as i32) {
                let team = if ci.team == Team::TeamBlue { "Pagans" } else { "Stroggs" };
                let fallback = self.model(
                    ci,
                    settings,
                    team_model,
                    &ci.skin_name.clone(),
                    team_head,
                    &ci.skin_name.clone(),
                    team,
                )?;
                if !fallback {
                    return Err(drop_msg(format!(
                        "DEFAULT_TEAM_MODEL / skin ({}/{}) failed to register",
                        team_model, ci.skin_name
                    )));
                }
            } else {
                let fallback = self.model(ci, settings, "sarge", "default", "sarge", "default", &team)?;
                if !fallback {
                    return Err(drop_msg("DEFAULT_MODEL (sarge) failed to register"));
                }
            }
        }
        let torso_model = ci.torso_model.clone();
        ci.new_anims = self.host.model_has_tag(&torso_model, "tag_flag");
        let fallback = custom_sound_fallback(product, (settings.game_type as i32) >= (GameType::GtTeam as i32));
        let mut sounds: [Option<PcmSound>; 32] = [None; 32];
        for (index, name) in CUSTOM_SOUND_NAMES.iter().enumerate() {
            let bare = name.strip_prefix('*').unwrap_or(name);
            sounds[index] = if loaded {
                self.host
                    .register_sound(&format!("sound/player/{}/{bare}", ci.model_name))?
            } else {
                None
            };
            if sounds[index].is_none() {
                sounds[index] = self.host.register_sound(&format!("sound/player/{fallback}/{bare}"))?;
            }
        }
        ci.sounds = sounds;
        ci.deferred = false;
        Ok(())
    }

    fn reuse(
        &mut self,
        slots: &[ClientInfo],
        ci: &mut ClientInfo,
        settings: &ClientInfoSettings,
    ) -> PresentResult<bool> {
        for index in 0..settings.max_clients {
            if index < 0 {
                continue;
            }
            let slot = at(slots, index as usize, "Player index")?;
            if slot.info_valid
                && !slot.deferred
                && same_fold(&ci.model_name, &slot.model_name)
                && same_fold(&ci.skin_name, &slot.skin_name)
                && same_fold(&ci.head_model_name, &slot.head_model_name)
                && same_fold(&ci.head_skin_name, &slot.head_skin_name)
                && same_fold(&ci.blue_team, &slot.blue_team)
                && same_fold(&ci.red_team, &slot.red_team)
                && ((settings.game_type as i32) < (GameType::GtTeam as i32) || ci.team == slot.team)
            {
                ci.deferred = false;
                let source = slot.clone();
                copy_client_model(&source, ci)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn defer(&mut self, slots: &[ClientInfo], ci: &mut ClientInfo, settings: &ClientInfoSettings) -> PresentResult<()> {
        let end = (settings.max_clients.max(0) as usize).min(slots.len());
        let candidates = &slots[..end];
        if candidates.iter().any(|slot| {
            slot.info_valid
                && !slot.deferred
                && same_fold(&ci.skin_name, &slot.skin_name)
                && same_fold(&ci.model_name, &slot.model_name)
                && ((settings.game_type as i32) < (GameType::GtTeam as i32) || ci.team == slot.team)
        }) {
            return self.load(ci, settings);
        }
        let found = candidates
            .iter()
            .find(|slot| {
                slot.info_valid
                    && ((settings.game_type as i32) < (GameType::GtTeam as i32)
                        || (!slot.deferred && same_fold(&ci.skin_name, &slot.skin_name) && ci.team == slot.team))
            })
            .cloned();
        if let Some(source) = found {
            ci.deferred = true;
            copy_client_model(&source, ci)?;
            return Ok(());
        }
        if (settings.game_type as i32) < (GameType::GtTeam as i32) {
            self.host.print("CG_SetDeferredClientInfo: no valid clients!\n");
        }
        self.load(ci, settings)
    }

    /// Parse a client configstring into a slot.
    pub fn new_client_info(&mut self, slots: &mut [ClientInfo], index: i32, configstring: &str) -> PresentResult<()> {
        if index < 0 || index >= MAX_CLIENTS as i32 {
            return Err(range_msg(format!("Player index {index} outside clients")));
        }
        let slot_index = index as usize;
        if slot_index >= slots.len() {
            return Err(range_msg(format!("Player index {index} outside clients")));
        }
        self.requests[slot_index] = self.requests[slot_index].wrapping_add(1);
        let mut next = ClientInfo::default();
        if configstring.is_empty() || configstring.starts_with('\0') {
            slots[slot_index] = next;
            return Ok(());
        }
        let settings = self.host.settings();
        let value = |key: &str| -> PresentResult<String> { info_value_for_key(configstring, key, 8192) };
        next.name = qpath_truncate(&value("n")?);
        next.color1 = info_color(&value("c1")?);
        next.color2 = info_color(&value("c2")?);
        next.bot_skill = game_atoi(&value("skill")?)?;
        next.handicap = game_atoi(&value("hc")?)?;
        next.wins = game_atoi(&value("w")?)?;
        next.losses = game_atoi(&value("l")?)?;
        let team = game_atoi(&value("t")?)?;
        if team != Team::TeamFree as i32
            && team != Team::TeamRed as i32
            && team != Team::TeamBlue as i32
            && team != Team::TeamSpectator as i32
        {
            return Err(range_msg(format!("Invalid player team {team}")));
        }
        next.team = Team::from_i32(team).unwrap_or(Team::TeamFree);
        next.team_task = game_atoi(&value("tt")?)?;
        next.team_leader = game_atoi(&value("tl")?)? != 0;
        next.red_team = value("g_redteam")?.chars().take(31).collect();
        next.blue_team = value("g_blueteam")?.chars().take(31).collect();
        let product = self.host.product();
        let forced = if product == Product::Missionpack {
            "james"
        } else {
            "sarge"
        };
        let (model_name, skin_name) = if settings.force_model {
            model_skin(if (settings.game_type as i32) >= (GameType::GtTeam as i32) {
                forced
            } else {
                &settings.model
            })
        } else {
            model_skin(&value("model")?)
        };
        next.model_name = model_name;
        next.skin_name = skin_name;
        let (head_model_name, head_skin_name) = if settings.force_model {
            model_skin(if (settings.game_type as i32) >= (GameType::GtTeam as i32) {
                forced
            } else {
                &settings.head_model
            })
        } else {
            model_skin(&value("hmodel")?)
        };
        next.head_model_name = head_model_name;
        next.head_skin_name = head_skin_name;
        if settings.force_model && (settings.game_type as i32) >= (GameType::GtTeam as i32) {
            let model = value("model")?;
            let head = value("hmodel")?;
            if let Some(slash) = model.find('/') {
                next.skin_name = qpath_truncate(&model[slash + 1..]);
            }
            if let Some(slash) = head.find('/') {
                next.head_skin_name = qpath_truncate(&head[slash + 1..]);
            }
        }
        if !self.reuse(slots, &mut next, &settings)? {
            let force_defer = self.host.memory_remaining() < 4_000_000;
            if force_defer || (settings.defer_players && !settings.build_script && !settings.loading) {
                self.defer(slots, &mut next, &settings)?;
                if force_defer {
                    self.host.print("Memory is low.  Using deferred model.\n");
                    next.deferred = false;
                }
            } else {
                self.load(&mut next, &settings)?;
            }
        }
        next.info_valid = true;
        slots[slot_index] = next;
        Ok(())
    }

    /// Load deferred players, resetting their entities.
    pub fn load_deferred_players(
        &mut self,
        state: &mut ClientGameState,
        slots: &mut [ClientInfo],
        reset: &mut dyn FnMut(&mut ClientEntity, &ClientInfo, i32) -> PresentResult<()>,
    ) -> PresentResult<()> {
        let lifecycle = self.lifecycle;
        let settings = self.host.settings();
        let time = state.time;
        let mut index = 0;
        while index < settings.max_clients && lifecycle == self.lifecycle {
            if index < 0 {
                index += 1;
                continue;
            }
            let slot_index = index as usize;
            if slot_index >= slots.len() {
                return Err(range_msg(format!("Player index {index} outside clients")));
            }
            if !slots[slot_index].info_valid || !slots[slot_index].deferred {
                index += 1;
                continue;
            }
            if slot_index < MAX_CLIENTS && self.host.memory_remaining() < 4_000_000 {
                self.host.print("Memory is low.  Using deferred model.\n");
                slots[slot_index].deferred = false;
                index += 1;
                continue;
            }
            if slot_index < MAX_CLIENTS {
                self.requests[slot_index] = self.requests[slot_index].wrapping_add(1);
            }
            let settings = self.host.settings();
            // The slot borrow ends before entity iteration below.
            let owned = std::mem::take(&mut slots[slot_index]);
            let mut owned = owned;
            self.load(&mut owned, &settings)?;
            slots[slot_index] = owned;
            for number in 0..MAX_ENTITIES as i32 {
                let entity = state.entity_at_mut(number)?;
                if entity.current_state.client_num == index
                    && entity.current_state.e_type == EntityType::EtPlayer as i32
                {
                    let ci = at(slots, slot_index, "Player index")?.clone();
                    reset(entity, &ci, time)?;
                }
            }
            index += 1;
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::q3::base::shared::entity_state::EntityState as CanonicalEntityState;
    use crate::q3::base::shared::player_state::PlayerState as CanonicalPlayerState;
    use crate::q3::base::world::{TraceContact, TraceSolidity};
    use crate::q3::presentation::client_info::ClientInfo;
    use crate::q3::presentation::mirrors_present_hud::{AnimationCell, ANIMATION_COUNT};
    use qa_core::math::{vec3, vec4, Plane, Vec3};
    use std::collections::HashMap;

    pub(crate) struct TestClientInfoHost {
        files: HashMap<String, Vec<u8>>,
        settings: ClientInfoSettings,
        product: Product,
        cache: HashMap<String, PcmSound>,
        prints: Vec<String>,
        parsed: Vec<String>,
        next_model: u32,
        next_skin: u32,
        next_shader: u32,
        next_sound: u32,
        memory: i64,
    }

    impl TestClientInfoHost {
        pub(crate) fn new() -> Self {
            Self {
                files: HashMap::new(),
                settings: ClientInfoSettings {
                    game_type: GameType::GtFfa,
                    max_clients: MAX_CLIENTS as i32,
                    force_model: false,
                    model: String::new(),
                    head_model: String::new(),
                    red_team_name: String::new(),
                    blue_team_name: String::new(),
                    defer_players: false,
                    build_script: false,
                    loading: false,
                },
                product: Product::Baseq3,
                cache: HashMap::new(),
                prints: Vec::new(),
                parsed: Vec::new(),
                next_model: 1,
                next_skin: 1,
                next_shader: 1,
                next_sound: 1,
                memory: i64::MAX,
            }
        }

        fn with_sarge_files() -> Self {
            let mut host = Self::new();
            for path in [
                "models/players/sarge/lower_default_default.skin",
                "models/players/sarge/upper_default_default.skin",
                "models/players/sarge/default/head_default.skin",
                "models/players/sarge/animation.cfg",
                "models/players/sarge/default/icon_default.skin",
            ] {
                host.files.insert(path.to_string(), b"data".to_vec());
            }
            host
        }
    }

    impl AssetReader for TestClientInfoHost {
        fn read_asset(&mut self, path: &str) -> PresentResult<Vec<u8>> {
            self.files
                .get(path)
                .cloned()
                .ok_or_else(|| state_msg(format!("asset not found: {path}")))
        }
        fn has_asset(&self, path: &str) -> bool {
            self.files.contains_key(path)
        }
        fn list_assets(&self, prefix: Option<&str>) -> Vec<String> {
            let mut names: Vec<String> = self
                .files
                .keys()
                .filter(|name| prefix.is_none_or(|prefix| name.starts_with(prefix)))
                .cloned()
                .collect();
            names.sort();
            names
        }
    }

    impl ClientInfoHost for TestClientInfoHost {
        fn product(&self) -> Product {
            self.product
        }
        fn settings(&self) -> ClientInfoSettings {
            self.settings.clone()
        }
        fn memory_remaining(&self) -> i64 {
            self.memory
        }
        fn register_model(&mut self, _path: &str) -> PresentResult<SceneModel> {
            let id = self.next_model;
            self.next_model += 1;
            Ok(SceneModel::Loaded { id })
        }
        fn register_skin(&mut self, _path: &str) -> PresentResult<Option<SceneSkin>> {
            let id = self.next_skin;
            self.next_skin += 1;
            Ok(Some(SceneSkin {
                id,
                surfaces: Vec::new(),
            }))
        }
        fn register_shader_no_mip(&mut self, name: &str) -> PresentResult<Option<SceneShader>> {
            let id = self.next_shader;
            self.next_shader += 1;
            Ok(Some(SceneShader {
                id,
                name: name.to_string(),
                material_order: id as i32,
            }))
        }
        fn register_sound(&mut self, _name: &str) -> PresentResult<Option<PcmSound>> {
            let id = self.next_sound;
            self.next_sound += 1;
            Ok(Some(PcmSound::new(id)))
        }
        fn sound(&self, name: &str) -> Option<PcmSound> {
            self.cache.get(name).copied()
        }
        fn parse_animation_config(&mut self, ci: &mut ClientInfo, _text: &str, path: &str) -> PresentResult<bool> {
            self.parsed.push(path.to_string());
            let dummy = AnimationCell {
                first_frame: 0,
                num_frames: 1,
                loop_frames: 0,
                frame_lerp: 100,
                initial_lerp: 100,
                reversed: false,
                flipflop: false,
            };
            ci.animations = [dummy; ANIMATION_COUNT];
            Ok(true)
        }
        fn model_has_tag(&mut self, _model: &SceneModel, tag: &str) -> bool {
            tag == "tag_flag"
        }
        fn print(&mut self, message: &str) {
            self.prints.push(message.to_string());
        }
    }

    // ---------- player doubles ----------
    fn player_media_fixture() -> PlayerMedia {
        PlayerMedia {
            connection_shader: None,
            balloon_shader: None,
            medal_impressive: None,
            medal_excellent: None,
            medal_gauntlet: None,
            medal_defend: None,
            medal_assist: None,
            medal_capture: None,
            friend_shader: None,
            shadow_mark_shader: None,
            wake_mark_shader: None,
            invis_shader: None,
            quad_shader: None,
            red_quad_shader: None,
            regen_shader: None,
            battle_suit_shader: None,
            haste_puff_shader: None,
            flight_sound: None,
            red_flag_model: default_model(),
            blue_flag_model: default_model(),
            neutral_flag_model: default_model(),
            flag_pole_model: default_model(),
            flag_flap_model: default_model(),
            red_flag_flap_skin: None,
            blue_flag_flap_skin: None,
            neutral_flag_flap_skin: None,
        }
    }
    fn player_settings_fixture() -> PlayerPresentationSettings {
        PlayerPresentationSettings {
            game_type: GameType::GtFfa,
            camera_mode: false,
            no_player_animations: true,
            animation_speed: 1.0,
            swing_speed: 1.0,
            draw_friend: false,
            shadows: 0,
            enable_breath: false,
            enable_dust: false,
            debug_position: false,
            debug_animation: false,
        }
    }
    pub(crate) struct TestPlayerHost {
        product: Product,
        media: PlayerMedia,
        settings: PlayerPresentationSettings,
        entities: Vec<RefEntity>,
        polys: Vec<RefPoly>,
        lights: Vec<DynamicLight>,
        puffs: Vec<(SmokePuffOptions, bool)>,
        loops: Vec<(i32, Vec3, Option<PcmSound>)>,
        weapons_added: u32,
        prints: Vec<String>,
        trace_result: TraceResult,
        contents: i32,
        random: i32,
        light: LightingSample,
        pose: PlayerPoseAxes,
    }

    impl TestPlayerHost {
        pub(crate) fn new() -> Self {
            let axis = [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)];
            Self {
                product: Product::Baseq3,
                media: player_media_fixture(),
                settings: player_settings_fixture(),
                entities: Vec::new(),
                polys: Vec::new(),
                lights: Vec::new(),
                puffs: Vec::new(),
                loops: Vec::new(),
                weapons_added: 0,
                prints: Vec::new(),
                trace_result: TraceResult {
                    fraction: 1.0,
                    end: vec3(0.0, 0.0, 0.0),
                    solidity: TraceSolidity::Clear,
                    contact: TraceContact::None,
                    contents: 0,
                    surface_flags: 0,
                },
                contents: 0,
                random: 0,
                light: LightingSample {
                    ambient_light: vec3(10.0, 10.0, 10.0),
                    directed_light: vec3(20.0, 20.0, 20.0),
                    light_dir: vec3(0.0, 0.0, 1.0),
                },
                pose: PlayerPoseAxes {
                    legs: axis,
                    torso: axis,
                    head: axis,
                },
            }
        }
    }

    impl PlayerPresentationHost for TestPlayerHost {
        fn product(&self) -> Product {
            self.product
        }
        fn media(&self) -> PlayerMedia {
            self.media.clone()
        }
        fn mission_media(&self) -> Option<MissionPlayerMedia> {
            None
        }
        fn settings(&self) -> PlayerPresentationSettings {
            self.settings.clone()
        }
        fn trace_world(&mut self, _start: Vec3, _end: Vec3, _mins: Vec3, _maxs: Vec3, _mask: i32) -> TraceResult {
            self.trace_result
        }
        fn trace_skip(
            &mut self,
            _start: Vec3,
            _end: Vec3,
            _mins: Vec3,
            _maxs: Vec3,
            _skip_number: i32,
            _mask: i32,
        ) -> TraceResult {
            self.trace_result
        }
        fn point_contents(&mut self, _point: Vec3) -> i32 {
            self.contents
        }
        fn add_entity(&mut self, entity: RefEntity) {
            self.entities.push(entity);
        }
        fn add_light(&mut self, light: DynamicLight) {
            self.lights.push(light);
        }
        fn add_poly(&mut self, poly: RefPoly) {
            self.polys.push(poly);
        }
        fn light_for_point(&mut self, _point: Vec3) -> LightingSample {
            self.light
        }
        fn impact_mark(&mut self, request: &ImpactMarkRequest) -> Vec<RefPoly> {
            vec![RefPoly {
                shader: request.shader.clone(),
                vertices: Vec::new(),
            }]
        }
        fn smoke_puff(&mut self, options: &SmokePuffOptions, scale_fade: bool) {
            self.puffs.push((options.clone(), scale_fade));
        }
        fn add_looping_sound(&mut self, entity_num: i32, origin: Vec3, _velocity: Vec3, sound: Option<PcmSound>) {
            self.loops.push((entity_num, origin, sound));
        }
        fn add_player_weapon(
            &mut self,
            _parent: &RefModelEntity,
            _ps: Option<&CanonicalPlayerState>,
            _entity: &ClientEntity,
            _team: Team,
        ) {
            self.weapons_added += 1;
        }
        fn random_int(&mut self) -> i32 {
            self.random
        }
        fn calculate_pose(
            &mut self,
            _player: &ClientPlayerEntity,
            _current: &CanonicalEntityState,
            _ci: &ClientInfo,
            _lerp_angles: Vec3,
            _time_ms: i32,
            _frame_time_ms: i32,
            _swing_speed: f32,
        ) -> PlayerPoseAxes {
            self.pose
        }
        fn clear_lerp_frame(
            &mut self,
            _ci: &ClientInfo,
            frame: &mut LerpFrame,
            animation: i32,
            time_ms: i32,
            _verbose: bool,
        ) {
            frame.animation_number = animation;
            frame.frame_time = time_ms;
            frame.old_frame_time = time_ms;
            frame.animation_time = time_ms;
        }
        fn run_lerp_frame(
            &mut self,
            _ci: &ClientInfo,
            frame: &mut LerpFrame,
            new_animation: i32,
            _speed_scale: f32,
            time_ms: i32,
            _frozen: bool,
            _verbose: bool,
        ) {
            frame.animation_number = new_animation;
            frame.frame = new_animation;
            frame.old_frame = new_animation;
            frame.frame_time = time_ms;
        }
        fn swing_angles(
            &mut self,
            destination: f32,
            _swing_tolerance: f32,
            _clamp_tolerance: f32,
            _speed: f32,
            _frame_time_ms: i32,
            _angle: f32,
            _swinging: bool,
        ) -> (f32, bool) {
            (destination, false)
        }
        fn position_on_tag(
            &mut self,
            entity: &mut RefModelEntity,
            parent: &RefModelEntity,
            _parent_model: &SceneModel,
            _tag: &str,
        ) -> PresentResult<()> {
            entity.origin = parent.origin;
            entity.axis = parent.axis;
            entity.back_lerp = parent.back_lerp;
            Ok(())
        }
        fn position_rotated_on_tag(
            &mut self,
            entity: &mut RefModelEntity,
            parent: &RefModelEntity,
            _parent_model: &SceneModel,
            _tag: &str,
        ) -> PresentResult<()> {
            entity.origin = parent.origin;
            Ok(())
        }
        fn print(&mut self, message: &str) {
            self.prints.push(message.to_string());
        }
    }

    // ---------- frame doubles ----------
    const CLIENT_CONFIG: &str = "\\n\\Newbie\\c1\\4\\c2\\3\\skill\\5\\hc\\100\\w\\7\\l\\2\\t\\0\\tt\\1\\tl\\0\\g_redteam\\redders\\g_blueteam\\blues\\model\\sarge\\hmodel\\sarge";
    #[test]
    fn client_info_empty_config_clears_slot() {
        let mut store = ClientInfoStore::new(TestClientInfoHost::new());
        let mut slots = vec![ClientInfo::default(); MAX_CLIENTS];
        slots[0].info_valid = true;
        store.new_client_info(&mut slots, 0, "").unwrap();
        assert!(!slots[0].info_valid);
        slots[0].info_valid = true;
        store.new_client_info(&mut slots, 0, "\0trailing").unwrap();
        assert!(!slots[0].info_valid);
    }

    #[test]
    fn client_info_rejects_bad_index_and_team() {
        let mut store = ClientInfoStore::new(TestClientInfoHost::new());
        let mut slots = vec![ClientInfo::default(); MAX_CLIENTS];
        assert!(store.new_client_info(&mut slots, 64, CLIENT_CONFIG).is_err());
        assert!(store.new_client_info(&mut slots, -1, CLIENT_CONFIG).is_err());
        assert!(store.new_client_info(&mut slots, 0, "\\t\\9").is_err());
    }

    #[test]
    fn client_info_loads_model_and_sounds() {
        let mut store = ClientInfoStore::new(TestClientInfoHost::with_sarge_files());
        let mut slots = vec![ClientInfo::default(); MAX_CLIENTS];
        store.new_client_info(&mut slots, 0, CLIENT_CONFIG).unwrap();
        let slot = &slots[0];
        assert!(slot.info_valid);
        assert_eq!(slot.name, "Newbie");
        assert_eq!(slot.color1, vec3(1.0, 0.0, 0.0));
        assert_eq!(slot.bot_skill, 5);
        assert_eq!(slot.team, Team::TeamFree);
        assert_eq!(slot.model_name, "sarge");
        assert_eq!(slot.skin_name, "default");
        assert!(slot.new_anims);
        assert!(!slot.legs_model.is_default());
        assert!(slot.model_icon.is_some());
        assert!(slot.sounds[0].is_some());
        assert!(slot.sounds[12].is_some());
        assert!(slot.sounds[13].is_none());
        assert!(!slot.deferred);
        assert!(store.host.prints.is_empty());
    }

    #[test]
    fn client_info_reuses_matching_model() {
        let mut store = ClientInfoStore::new(TestClientInfoHost::with_sarge_files());
        let mut slots = vec![ClientInfo::default(); MAX_CLIENTS];
        store.new_client_info(&mut slots, 0, CLIENT_CONFIG).unwrap();
        let before = store.host.next_model;
        store.new_client_info(&mut slots, 1, CLIENT_CONFIG).unwrap();
        assert!(slots[1].info_valid);
        assert!(!slots[1].deferred);
        assert_eq!(slots[1].legs_model, slots[0].legs_model);
        assert_eq!(store.host.next_model, before);
    }

    #[test]
    fn client_info_defers_without_match() {
        let mut host = TestClientInfoHost::with_sarge_files();
        host.settings.defer_players = true;
        let mut store = ClientInfoStore::new(host);
        let mut slots = vec![ClientInfo::default(); MAX_CLIENTS];
        slots[0].info_valid = true;
        slots[0].model_name = "other".to_string();
        slots[0].skin_name = "other".to_string();
        slots[0].head_model_name = "other".to_string();
        slots[0].head_skin_name = "other".to_string();
        let dummy = AnimationCell {
            first_frame: 0,
            num_frames: 1,
            loop_frames: 0,
            frame_lerp: 100,
            initial_lerp: 100,
            reversed: false,
            flipflop: false,
        };
        slots[0].animations = [dummy; ANIMATION_COUNT];
        store.new_client_info(&mut slots, 1, CLIENT_CONFIG).unwrap();
        assert!(slots[1].info_valid);
        assert!(slots[1].deferred);
        assert_eq!(slots[1].model_name, "sarge");
    }

    #[test]
    fn client_info_custom_sounds() {
        let mut host = TestClientInfoHost::new();
        host.cache.insert("sound/x.wav".to_string(), PcmSound::new(7));
        let mut store = ClientInfoStore::new(host);
        let mut slots = vec![ClientInfo::default(); MAX_CLIENTS];
        slots[0].sounds[0] = Some(PcmSound::new(1));
        assert_eq!(
            store.custom_sound(&slots, 0, "sound/x.wav").unwrap(),
            Some(PcmSound::new(7))
        );
        assert_eq!(store.custom_sound(&slots, 0, "sound/missing.wav").unwrap(), None);
        assert_eq!(
            store.custom_sound(&slots, 0, "*death1.wav").unwrap(),
            Some(PcmSound::new(1))
        );
        assert_eq!(
            store.custom_sound(&slots, 99, "*death1.wav").unwrap(),
            Some(PcmSound::new(1))
        );
        assert!(matches!(
            store.custom_sound(&slots, 0, "*nope.wav"),
            Err(PresentClientError::Drop(_))
        ));
    }

    #[test]
    fn custom_sound_fallback_names() {
        assert_eq!(custom_sound_fallback(Product::Missionpack, true), "james");
        assert_eq!(custom_sound_fallback(Product::Missionpack, false), "sarge");
        assert_eq!(custom_sound_fallback(Product::Baseq3, true), "sarge");
    }

    // ---------- presenter tests ----------

    fn render_fixture() -> (Vec<ClientInfo>, [SkullTrail; MAX_CLIENTS], ClientEntity) {
        let clients = vec![ClientInfo::default(); MAX_CLIENTS];
        let trails: [SkullTrail; MAX_CLIENTS] = std::array::from_fn(|_| SkullTrail {
            positions: [vec3(0.0, 0.0, 0.0); 10],
            num_positions: 0,
        });
        let mut entity = ClientEntity::new();
        entity.current_state.number = 7;
        entity.current_state.e_type = EntityType::EtPlayer as i32;
        entity.current_state.client_num = 0;
        (clients, trails, entity)
    }

    #[test]
    fn presenter_rejects_product_mismatch() {
        let mut host = TestPlayerHost::new();
        host.product = Product::Missionpack;
        assert!(PlayerPresenter::new(host, Product::Baseq3).is_err());
    }

    #[test]
    fn presenter_rejects_bad_client_number() {
        let host = TestPlayerHost::new();
        let mut presenter = PlayerPresenter::new(host, Product::Baseq3).unwrap();
        let (mut clients, mut trails, mut entity) = render_fixture();
        entity.current_state.client_num = 99;
        let mut ctx = PlayerRenderContext {
            time: 1000,
            frame_time: 50,
            rendering_third_person: true,
            snapshot_client_num: 3,
            snapshot_team: Team::TeamRed as i32,
            clients: &mut clients,
            skull_trails: &mut trails,
        };
        assert!(matches!(
            presenter.player(&mut ctx, &mut entity),
            Err(PresentClientError::Drop(_))
        ));
    }

    #[test]
    fn presenter_skips_invalid_info() {
        let host = TestPlayerHost::new();
        let mut presenter = PlayerPresenter::new(host, Product::Baseq3).unwrap();
        let (mut clients, mut trails, mut entity) = render_fixture();
        let mut ctx = PlayerRenderContext {
            time: 1000,
            frame_time: 50,
            rendering_third_person: true,
            snapshot_client_num: 3,
            snapshot_team: Team::TeamRed as i32,
            clients: &mut clients,
            skull_trails: &mut trails,
        };
        presenter.player(&mut ctx, &mut entity).unwrap();
        assert!(presenter.host.entities.is_empty());
        assert_eq!(presenter.host.weapons_added, 0);
    }

    #[test]
    fn presenter_renders_body_and_weapon() {
        let host = TestPlayerHost::new();
        let mut presenter = PlayerPresenter::new(host, Product::Baseq3).unwrap();
        let (mut clients, mut trails, mut entity) = render_fixture();
        clients[0].info_valid = true;
        clients[0].team = Team::TeamRed;
        clients[0].legs_model = SceneModel::Loaded { id: 1 };
        clients[0].torso_model = SceneModel::Loaded { id: 2 };
        clients[0].head_model = SceneModel::Loaded { id: 3 };
        let mut ctx = PlayerRenderContext {
            time: 1000,
            frame_time: 50,
            rendering_third_person: true,
            snapshot_client_num: 3,
            snapshot_team: Team::TeamRed as i32,
            clients: &mut clients,
            skull_trails: &mut trails,
        };
        presenter.player(&mut ctx, &mut entity).unwrap();
        assert_eq!(presenter.host.entities.len(), 3);
        assert_eq!(presenter.host.weapons_added, 1);
        assert!(presenter.host.polys.is_empty());
    }

    #[test]
    fn presenter_quad_adds_shell_and_light() {
        let mut host = TestPlayerHost::new();
        host.media.quad_shader = Some(SceneShader {
            id: 9,
            name: "quad".to_string(),
            material_order: 9,
        });
        let mut presenter = PlayerPresenter::new(host, Product::Baseq3).unwrap();
        let (mut clients, mut trails, mut entity) = render_fixture();
        clients[0].info_valid = true;
        clients[0].team = Team::TeamRed;
        clients[0].legs_model = SceneModel::Loaded { id: 1 };
        clients[0].torso_model = SceneModel::Loaded { id: 2 };
        clients[0].head_model = SceneModel::Loaded { id: 3 };
        entity.current_state.powerups = 1 << (Powerup::PwQuad as i32);
        let mut ctx = PlayerRenderContext {
            time: 1000,
            frame_time: 50,
            rendering_third_person: true,
            snapshot_client_num: 3,
            snapshot_team: Team::TeamRed as i32,
            clients: &mut clients,
            skull_trails: &mut trails,
        };
        presenter.player(&mut ctx, &mut entity).unwrap();
        assert_eq!(presenter.host.entities.len(), 6);
        assert_eq!(presenter.host.lights.len(), 1);
        assert_eq!(presenter.host.lights[0].color, vec3(0.2, 0.2, 1.0));
    }

    #[test]
    fn presenter_shadow_reports_mark_plane() {
        let mut host = TestPlayerHost::new();
        host.settings.shadows = 1;
        host.trace_result = TraceResult {
            fraction: 0.5,
            end: vec3(0.0, 0.0, 10.0),
            solidity: TraceSolidity::Clear,
            contact: TraceContact::Plane {
                plane: Plane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 10.0,
                },
            },
            contents: 0,
            surface_flags: 0,
        };
        let mut presenter = PlayerPresenter::new(host, Product::Baseq3).unwrap();
        let (mut clients, mut trails, mut entity) = render_fixture();
        clients[0].info_valid = true;
        clients[0].team = Team::TeamRed;
        clients[0].legs_model = SceneModel::Loaded { id: 1 };
        clients[0].torso_model = SceneModel::Loaded { id: 2 };
        clients[0].head_model = SceneModel::Loaded { id: 3 };
        let mut ctx = PlayerRenderContext {
            time: 1000,
            frame_time: 50,
            rendering_third_person: true,
            snapshot_client_num: 3,
            snapshot_team: Team::TeamRed as i32,
            clients: &mut clients,
            skull_trails: &mut trails,
        };
        presenter.player(&mut ctx, &mut entity).unwrap();
        assert_eq!(presenter.host.polys.len(), 1);
        match &presenter.host.entities[0] {
            RefEntity::Model(legs) => assert_eq!(legs.shadow_plane, 11.0),
            RefEntity::Sprite(_) => panic!("legs must render as a model"),
        }
    }

    #[test]
    fn presenter_reset_rebuilds_lerp() {
        let mut host = TestPlayerHost::new();
        host.settings.debug_position = true;
        let mut presenter = PlayerPresenter::new(host, Product::Baseq3).unwrap();
        let mut entity = ClientEntity::new();
        entity.current_state.pos.base = vec3(10.0, 20.0, 30.0);
        let ci = ClientInfo::default();
        presenter.reset_player_entity(1000, &mut entity, &ci).unwrap();
        assert_eq!(entity.error_time, -99999);
        assert!(!entity.extrapolated);
        assert_eq!(entity.lerp_origin, vec3(10.0, 20.0, 30.0));
        assert_eq!(entity.raw_origin, vec3(10.0, 20.0, 30.0));
        assert_eq!(entity.player.legs.base.animation_number, 0);
        assert_eq!(presenter.host.prints.len(), 1);
        assert!(presenter.host.prints[0].contains("ResetPlayerEntity"));
    }

    #[test]
    fn presenter_light_verts_requires_vertices() {
        let host = TestPlayerHost::new();
        let mut presenter = PlayerPresenter::new(host, Product::Baseq3).unwrap();
        assert!(presenter.light_verts(vec3(0.0, 0.0, 1.0), &mut []).is_err());
        let mut vertices = vec![RefPolyVertex {
            position: vec3(0.0, 0.0, 0.0),
            tex_coord: [0.0, 0.0],
            color: vec4(0.0, 0.0, 0.0, 0.0),
        }];
        assert!(presenter.light_verts(vec3(0.0, 0.0, 1.0), &mut vertices).unwrap());
        assert_eq!(vertices[0].color, vec4(30.0, 30.0, 30.0, 255.0));
        assert!(presenter.light_verts(vec3(0.0, 0.0, -1.0), &mut vertices).unwrap());
        assert_eq!(vertices[0].color, vec4(10.0, 10.0, 10.0, 255.0));
    }

    // ---------- frame tests ----------
}
