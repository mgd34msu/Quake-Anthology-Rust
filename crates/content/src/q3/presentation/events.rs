//! Quake III presentation: events.
//!
//! Donor provenance: `src/content/q3/presentation/events.ts`.

use crate::q3anim::{PlayerFootsteps, PlayerGender};
use qa_core::math::{vec3, vec4, Vec3, Vec4};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::format::{game_format_bounded, GameFormatArgument};
use crate::q3::base::shared::definitions::*;
use crate::q3::base::shared::direction_byte::byte_to_direction;
use crate::q3::base::shared::entity_state::*;
use crate::q3::base::shared::items::{find_item_for_holdable, item_at, item_list, ItemKind};
use crate::q3::base::shared::player_state::*;
use crate::q3::base::shared::trajectory::evaluate_trajectory;
use crate::q3::presentation::retail_snapshot::*;
use crate::q3::presentation::state::*;

// ---------------------------------------------------------------------------
// Entity events (events.ts)
// ---------------------------------------------------------------------------

pub(crate) const EV_AUTO: i32 = 0;

pub(crate) const EV_VOICE: i32 = 3;

pub(crate) const EV_ITEM: i32 = 4;

pub(crate) const EV_BODY: i32 = 5;

pub(crate) const EV_ANNOUNCER: i32 = 7;

/// Player-state event sounds (`ClientEventSound` record).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClientEventSounds {
    /// Use-nothing sound.
    pub use_nothing_sound: Option<PcmSound>,
    /// Medkit sound.
    pub medkit_sound: Option<PcmSound>,
    /// Land sound.
    pub land_sound: Option<PcmSound>,
    /// Jump-pad sound.
    pub jump_pad_sound: Option<PcmSound>,
    /// Water-in sound.
    pub watr_in_sound: Option<PcmSound>,
    /// Water-out sound.
    pub watr_out_sound: Option<PcmSound>,
    /// Water-under sound.
    pub watr_un_sound: Option<PcmSound>,
    /// Health sound.
    pub n_health_sound: Option<PcmSound>,
    /// Select sound.
    pub select_sound: Option<PcmSound>,
    /// Teleport-in sound.
    pub tele_in_sound: Option<PcmSound>,
    /// Teleport-out sound.
    pub tele_out_sound: Option<PcmSound>,
    /// Respawn sound.
    pub respawn_sound: Option<PcmSound>,
    /// Grenade bounce 1.
    pub hgrenb1a_sound: Option<PcmSound>,
    /// Grenade bounce 2.
    pub hgrenb2a_sound: Option<PcmSound>,
    /// Capture-your-team sound.
    pub capture_your_team_sound: Option<PcmSound>,
    /// Capture-opponent sound.
    pub capture_opponent_sound: Option<PcmSound>,
    /// Return-your-team sound.
    pub return_your_team_sound: Option<PcmSound>,
    /// Return-opponent sound.
    pub return_opponent_sound: Option<PcmSound>,
    /// Blue-flag-returned sound.
    pub blue_flag_returned_sound: Option<PcmSound>,
    /// Red-flag-returned sound.
    pub red_flag_returned_sound: Option<PcmSound>,
    /// Enemy-took-your-flag sound.
    pub enemy_took_your_flag_sound: Option<PcmSound>,
    /// Your-team-took-enemy-flag sound.
    pub your_team_took_enemy_flag_sound: Option<PcmSound>,
    /// Your-base-under-attack sound.
    pub your_base_is_under_attack_sound: Option<PcmSound>,
    /// Red-scored sound.
    pub red_scored_sound: Option<PcmSound>,
    /// Blue-scored sound.
    pub blue_scored_sound: Option<PcmSound>,
    /// Red-leads sound.
    pub red_leads_sound: Option<PcmSound>,
    /// Blue-leads sound.
    pub blue_leads_sound: Option<PcmSound>,
    /// Teams-tied sound.
    pub teams_tied_sound: Option<PcmSound>,
    /// Quad sound.
    pub quad_sound: Option<PcmSound>,
    /// Protect sound.
    pub protect_sound: Option<PcmSound>,
    /// Regen sound.
    pub regen_sound: Option<PcmSound>,
    /// Gib sound.
    pub gib_sound: Option<PcmSound>,
}

/// Mission-pack event sounds (`MissionEventSound` record).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MissionEventSounds {
    /// Use-invulnerability sound.
    pub use_invulnerability_sound: Option<PcmSound>,
    /// Scout sound.
    pub scout_sound: Option<PcmSound>,
    /// Guard sound.
    pub guard_sound: Option<PcmSound>,
    /// Doubler sound.
    pub doubler_sound: Option<PcmSound>,
    /// Ammo-regen sound.
    pub ammoregen_sound: Option<PcmSound>,
    /// Prox-mine stick (flesh).
    pub wstbimpl_sound: Option<PcmSound>,
    /// Prox-mine stick (metal).
    pub wstbimpm_sound: Option<PcmSound>,
    /// Prox-mine stick (default).
    pub wstbimpd_sound: Option<PcmSound>,
    /// Prox-mine trigger.
    pub wstbactv_sound: Option<PcmSound>,
    /// Your-team-took-the-flag sound.
    pub your_team_took_the_flag_sound: Option<PcmSound>,
    /// Enemy-took-the-flag sound.
    pub enemy_took_the_flag_sound: Option<PcmSound>,
    /// Kamikaze-far sound.
    pub kamikaze_far_sound: Option<PcmSound>,
}

/// Footstep sound bank keyed by surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FootstepBank {
    /// Normal.
    pub normal: [Option<PcmSound>; 4],
    /// Boot.
    pub boot: [Option<PcmSound>; 4],
    /// Flesh.
    pub flesh: [Option<PcmSound>; 4],
    /// Mech.
    pub mech: [Option<PcmSound>; 4],
    /// Energy.
    pub energy: [Option<PcmSound>; 4],
    /// Metal.
    pub metal: [Option<PcmSound>; 4],
    /// Splash.
    pub splash: [Option<PcmSound>; 4],
}

impl FootstepBank {
    /// Bank for a footstep style.
    #[must_use]
    pub const fn bank(&self, footsteps: PlayerFootsteps) -> &[Option<PcmSound>; 4] {
        match footsteps {
            PlayerFootsteps::Normal => &self.normal,
            PlayerFootsteps::Boot => &self.boot,
            PlayerFootsteps::Flesh => &self.flesh,
            PlayerFootsteps::Mech => &self.mech,
            PlayerFootsteps::Energy => &self.energy,
        }
    }
}

/// Client event media (`ClientEventMedia`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientEventMedia {
    /// Sounds.
    pub sounds: ClientEventSounds,
    /// Footsteps.
    pub footsteps: FootstepBank,
    /// Game sounds.
    pub game_sounds: Vec<Option<PcmSound>>,
    /// Smoke-puff shader.
    pub smoke_puff_shader: Option<SceneShader>,
}

/// Client event options (`ClientEventOptions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientEventOptions {
    /// Game type.
    pub game_type: GameType,
    /// Debug events.
    pub debug_events: bool,
    /// Footsteps enabled.
    pub footsteps: bool,
    /// Autoswitch.
    pub autoswitch: bool,
    /// Demo playback.
    pub demo_playback: bool,
    /// No predict.
    pub no_predict: bool,
    /// Synchronous clients.
    pub synchronous_clients: bool,
    /// Single-player active.
    pub single_player_active: bool,
    /// Camera orbit.
    pub camera_orbit: bool,
}

/// Brief client info for events.
#[derive(Debug, Clone, Copy)]
pub struct EventClientInfo {
    /// Gender.
    pub gender: PlayerGender,
    /// Footsteps.
    pub footsteps: PlayerFootsteps,
    /// Team.
    pub team: Team,
}

/// Missile impact sound (`ImpactSound`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImpactSound {
    /// Default.
    Default,
    /// Metal.
    Metal,
}

/// Bullet target (`weapons.bullet` target).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BulletTarget {
    /// Wall hit with normal.
    Wall {
        /// Normal.
        normal: Vec3,
    },
    /// Flesh hit with entity number.
    Flesh {
        /// Entity number.
        entity_num: i32,
    },
}

/// Smoke-puff descriptor.
#[derive(Debug, Clone, PartialEq)]
pub struct SmokePuffDesc {
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Radius.
    pub radius: f32,
    /// Color.
    pub color: Vec4,
    /// Duration.
    pub duration: i32,
    /// Start time.
    pub start_time: i32,
    /// Fade-in time.
    pub fade_in_time: i32,
    /// Flags.
    pub flags: i32,
    /// Shader.
    pub shader: Option<SceneShader>,
}

/// Spawned smoke puff handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnedPuff {
    /// Local-entity type override.
    pub le_type: Option<String>,
}

/// Entity event services (`ClientEventHost`).
#[allow(clippy::too_many_arguments)]
pub trait ClientEventHost {
    /// Product.
    fn product(&self) -> Product;
    /// Media.
    fn media(&self) -> &ClientEventMedia;
    /// Options.
    fn options(&self) -> ClientEventOptions;
    /// Mission sounds.
    fn mission_sounds(&self) -> Option<&MissionEventSounds>;
    /// Random integer (`rand`).
    fn rand_int(&mut self) -> i32;
    /// Recipe pre-hook; `false` skips the source event.
    fn pre_present_event(&mut self, state: &mut ClientGameState, entity_ref: EventEntityRef, position: Vec3) -> bool;
    /// Recipe post-hook.
    fn post_present_event(&mut self, state: &mut ClientGameState, entity_ref: EventEntityRef, position: Vec3);
    /// Set the entity sound position.
    fn set_entity_sound_position(&mut self, state: &ClientGameState, entity_number: usize);
    /// Debug beam.
    fn beam(&mut self, state: &mut ClientGameState, entity_ref: EventEntityRef);
    /// Out-of-ammo weapon change.
    fn out_of_ammo_change(&mut self, state: &mut ClientGameState);
    /// Fire weapon effect.
    fn fire_weapon(&mut self, state: &mut ClientGameState, entity_ref: EventEntityRef);
    /// Missile-hit-player effect.
    fn missile_hit_player(
        &mut self,
        state: &mut ClientGameState,
        weapon: i32,
        position: Vec3,
        direction: Vec3,
        other_entity_num: i32,
    );
    /// Missile-hit-wall effect.
    fn missile_hit_wall(
        &mut self,
        state: &mut ClientGameState,
        weapon: i32,
        client_num: i32,
        position: Vec3,
        direction: Vec3,
        impact: ImpactSound,
    );
    /// Rail trail effect.
    fn rail_trail(&mut self, state: &mut ClientGameState, client_num: i32, origin2: Vec3, base: Vec3);
    /// Bullet effect.
    fn bullet(&mut self, state: &mut ClientGameState, base: Vec3, other_entity_num: i32, target: BulletTarget);
    /// Shotgun effect.
    fn shotgun_fire(&mut self, state: &mut ClientGameState, entity_ref: EventEntityRef);
    /// Smoke puff.
    fn smoke_puff(
        &mut self,
        state: &mut ClientGameState,
        desc: &SmokePuffDesc,
        le_type_override: Option<&str>,
    ) -> SpawnedPuff;
    /// Spawn effect.
    fn spawn_effect(&mut self, state: &mut ClientGameState, position: Vec3);
    /// Gib effect.
    fn gib_player(&mut self, state: &mut ClientGameState, position: Vec3);
    /// Score plum.
    fn score_plum(&mut self, state: &mut ClientGameState, other_entity_num: i32, position: Vec3, time: i32);
    /// Kamikaze effect.
    fn kamikaze_effect(&mut self, state: &mut ClientGameState, position: Vec3);
    /// Obelisk explosion.
    fn obelisk_explode(&mut self, state: &mut ClientGameState, position: Vec3);
    /// Obelisk pain.
    fn obelisk_pain(&mut self, state: &mut ClientGameState, position: Vec3);
    /// Invulnerability impact.
    fn invulnerability_impact(&mut self, state: &mut ClientGameState, position: Vec3, angles: Vec3);
    /// Invulnerability juiced.
    fn invulnerability_juiced(&mut self, state: &mut ClientGameState, position: Vec3);
    /// Lightning-bolt beam.
    fn lightning_bolt_beam(&mut self, state: &mut ClientGameState, origin2: Vec3, base: Vec3);
    /// Brief client info.
    fn client_info_brief(&self, static_state: &ClientGameStaticState, number: i32) -> PresentResult<EventClientInfo>;
    /// Record medkit usage time.
    fn set_medkit_usage_time(
        &mut self,
        static_state: &mut ClientGameStaticState,
        client_num: i32,
        time: i32,
    ) -> PresentResult<()>;
    /// Live player name, if the configstring exists.
    fn player_name(&self, number: i32) -> Option<String>;
    /// Sound configstring.
    fn sound_config_string(&self, index: i32) -> String;
    /// Custom sound.
    fn custom_sound(&mut self, client_num: i32, name: &str) -> Option<PcmSound>;
    /// Register a sound synchronously.
    fn register_sound(&mut self, path: Option<&str>, compressed: bool) -> Option<PcmSound>;
    /// Start a sound.
    fn start_sound(&mut self, origin: Option<Vec3>, entity_num: i32, channel: i32, sound: Option<PcmSound>);
    /// Start a local sound.
    fn start_local_sound(&mut self, sound: Option<PcmSound>, channel: i32);
    /// Stop a looping sound.
    fn stop_looping_sound(&mut self, entity_num: i32);
    /// Buffer a sound.
    fn add_buffered_sound(&mut self, sound: Option<PcmSound>);
    /// Print.
    fn print(&mut self, message: &str);
    /// Center print.
    fn center_print(&mut self, message: &str, y: i32, char_width: i32);
    /// Local voice chat.
    fn voice_chat_local(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        mode: i32,
        voice_only: bool,
        client_num: i32,
        color: i32,
        command: &str,
    ) -> PresentResult<()>;
}

/// Rank place string (`placeString`).
pub fn place_string(rank: i32) -> String {
    let tied = (rank & 0x4000) != 0;
    let rank = rank & !0x4000;
    let place = if rank == 1 {
        "^41st^7".to_owned()
    } else if rank == 2 {
        "^12nd^7".to_owned()
    } else if rank == 3 {
        "^33rd^7".to_owned()
    } else {
        let suffix = if rank == 11 || rank == 12 || rank == 13 {
            "th"
        } else if rank % 10 == 1 {
            "st"
        } else if rank % 10 == 2 {
            "nd"
        } else if rank % 10 == 3 {
            "rd"
        } else {
            "th"
        };
        game_format_bounded(&format!("%i{suffix}"), &[GameFormatArgument::from(rank)], 16_384)
    };
    game_format_bounded(
        "%s%s",
        &[
            GameFormatArgument::from(if tied { "Tied for " } else { "" }),
            GameFormatArgument::from(place),
        ],
        64,
    )
}

/// Canonical entity event from a wire tag (`from_i32`).
fn entity_event_from_i32(event: i32) -> Option<EntityEvent> {
    match event {
        0 => Some(EntityEvent::EvNone),
        1 => Some(EntityEvent::EvFootstep),
        2 => Some(EntityEvent::EvFootstepMetal),
        3 => Some(EntityEvent::EvFootsplash),
        4 => Some(EntityEvent::EvFootwade),
        5 => Some(EntityEvent::EvSwim),
        6 => Some(EntityEvent::EvStep4),
        7 => Some(EntityEvent::EvStep8),
        8 => Some(EntityEvent::EvStep12),
        9 => Some(EntityEvent::EvStep16),
        10 => Some(EntityEvent::EvFallShort),
        11 => Some(EntityEvent::EvFallMedium),
        12 => Some(EntityEvent::EvFallFar),
        13 => Some(EntityEvent::EvJumpPad),
        14 => Some(EntityEvent::EvJump),
        15 => Some(EntityEvent::EvWaterTouch),
        16 => Some(EntityEvent::EvWaterLeave),
        17 => Some(EntityEvent::EvWaterUnder),
        18 => Some(EntityEvent::EvWaterClear),
        19 => Some(EntityEvent::EvItemPickup),
        20 => Some(EntityEvent::EvGlobalItemPickup),
        21 => Some(EntityEvent::EvNoammo),
        22 => Some(EntityEvent::EvChangeWeapon),
        23 => Some(EntityEvent::EvFireWeapon),
        24 => Some(EntityEvent::EvUseItem0),
        25 => Some(EntityEvent::EvUseItem1),
        26 => Some(EntityEvent::EvUseItem2),
        27 => Some(EntityEvent::EvUseItem3),
        28 => Some(EntityEvent::EvUseItem4),
        29 => Some(EntityEvent::EvUseItem5),
        30 => Some(EntityEvent::EvUseItem6),
        31 => Some(EntityEvent::EvUseItem7),
        32 => Some(EntityEvent::EvUseItem8),
        33 => Some(EntityEvent::EvUseItem9),
        34 => Some(EntityEvent::EvUseItem10),
        35 => Some(EntityEvent::EvUseItem11),
        36 => Some(EntityEvent::EvUseItem12),
        37 => Some(EntityEvent::EvUseItem13),
        38 => Some(EntityEvent::EvUseItem14),
        39 => Some(EntityEvent::EvUseItem15),
        40 => Some(EntityEvent::EvItemRespawn),
        41 => Some(EntityEvent::EvItemPop),
        42 => Some(EntityEvent::EvPlayerTeleportIn),
        43 => Some(EntityEvent::EvPlayerTeleportOut),
        44 => Some(EntityEvent::EvGrenadeBounce),
        45 => Some(EntityEvent::EvGeneralSound),
        46 => Some(EntityEvent::EvGlobalSound),
        47 => Some(EntityEvent::EvGlobalTeamSound),
        48 => Some(EntityEvent::EvBulletHitFlesh),
        49 => Some(EntityEvent::EvBulletHitWall),
        50 => Some(EntityEvent::EvMissileHit),
        51 => Some(EntityEvent::EvMissileMiss),
        52 => Some(EntityEvent::EvMissileMissMetal),
        53 => Some(EntityEvent::EvRailtrail),
        54 => Some(EntityEvent::EvShotgun),
        55 => Some(EntityEvent::EvBullet),
        56 => Some(EntityEvent::EvPain),
        57 => Some(EntityEvent::EvDeath1),
        58 => Some(EntityEvent::EvDeath2),
        59 => Some(EntityEvent::EvDeath3),
        60 => Some(EntityEvent::EvObituary),
        61 => Some(EntityEvent::EvPowerupQuad),
        62 => Some(EntityEvent::EvPowerupBattlesuit),
        63 => Some(EntityEvent::EvPowerupRegen),
        64 => Some(EntityEvent::EvGibPlayer),
        65 => Some(EntityEvent::EvScoreplum),
        66 => Some(EntityEvent::EvProximityMineStick),
        67 => Some(EntityEvent::EvProximityMineTrigger),
        68 => Some(EntityEvent::EvKamikaze),
        69 => Some(EntityEvent::EvObeliskexplode),
        70 => Some(EntityEvent::EvObeliskpain),
        71 => Some(EntityEvent::EvInvulImpact),
        72 => Some(EntityEvent::EvJuiced),
        73 => Some(EntityEvent::EvLightningbolt),
        74 => Some(EntityEvent::EvDebugLine),
        75 => Some(EntityEvent::EvStoploopingsound),
        76 => Some(EntityEvent::EvTaunt),
        77 => Some(EntityEvent::EvTauntYes),
        78 => Some(EntityEvent::EvTauntNo),
        79 => Some(EntityEvent::EvTauntFollowme),
        80 => Some(EntityEvent::EvTauntGetflag),
        81 => Some(EntityEvent::EvTauntGuardbase),
        82 => Some(EntityEvent::EvTauntPatrol),
        _ => None,
    }
}

/// Canonical holdable from a use-item tag.
fn holdable_from_tag(tag: i32) -> PresentResult<Holdable> {
    match tag {
        1 => Ok(Holdable::HiTeleporter),
        2 => Ok(Holdable::HiMedkit),
        3 => Ok(Holdable::HiKamikaze),
        4 => Ok(Holdable::HiPortal),
        5 => Ok(Holdable::HiInvulnerability),
        _ => Err(drop_msg("HoldableItem not found")),
    }
}

pub(crate) fn entity_event_name(event: i32) -> Option<&'static str> {
    entity_event_from_i32(event).map(|event| match event {
        EntityEvent::EvNone => "EV_NONE",
        EntityEvent::EvFootstep => "EV_FOOTSTEP",
        EntityEvent::EvFootstepMetal => "EV_FOOTSTEP_METAL",
        EntityEvent::EvFootsplash => "EV_FOOTSPLASH",
        EntityEvent::EvFootwade => "EV_FOOTWADE",
        EntityEvent::EvSwim => "EV_SWIM",
        EntityEvent::EvStep4 => "EV_STEP_4",
        EntityEvent::EvStep8 => "EV_STEP_8",
        EntityEvent::EvStep12 => "EV_STEP_12",
        EntityEvent::EvStep16 => "EV_STEP_16",
        EntityEvent::EvFallShort => "EV_FALL_SHORT",
        EntityEvent::EvFallMedium => "EV_FALL_MEDIUM",
        EntityEvent::EvFallFar => "EV_FALL_FAR",
        EntityEvent::EvJumpPad => "EV_JUMP_PAD",
        EntityEvent::EvJump => "EV_JUMP",
        EntityEvent::EvWaterTouch => "EV_WATER_TOUCH",
        EntityEvent::EvWaterLeave => "EV_WATER_LEAVE",
        EntityEvent::EvWaterUnder => "EV_WATER_UNDER",
        EntityEvent::EvWaterClear => "EV_WATER_CLEAR",
        EntityEvent::EvItemPickup => "EV_ITEM_PICKUP",
        EntityEvent::EvGlobalItemPickup => "EV_GLOBAL_ITEM_PICKUP",
        EntityEvent::EvNoammo => "EV_NOAMMO",
        EntityEvent::EvChangeWeapon => "EV_CHANGE_WEAPON",
        EntityEvent::EvFireWeapon => "EV_FIRE_WEAPON",
        EntityEvent::EvUseItem0 => "EV_USE_ITEM0",
        EntityEvent::EvUseItem1 => "EV_USE_ITEM1",
        EntityEvent::EvUseItem2 => "EV_USE_ITEM2",
        EntityEvent::EvUseItem3 => "EV_USE_ITEM3",
        EntityEvent::EvUseItem4 => "EV_USE_ITEM4",
        EntityEvent::EvUseItem5 => "EV_USE_ITEM5",
        EntityEvent::EvUseItem6 => "EV_USE_ITEM6",
        EntityEvent::EvUseItem7 => "EV_USE_ITEM7",
        EntityEvent::EvUseItem8 => "EV_USE_ITEM8",
        EntityEvent::EvUseItem9 => "EV_USE_ITEM9",
        EntityEvent::EvUseItem10 => "EV_USE_ITEM10",
        EntityEvent::EvUseItem11 => "EV_USE_ITEM11",
        EntityEvent::EvUseItem12 => "EV_USE_ITEM12",
        EntityEvent::EvUseItem13 => "EV_USE_ITEM13",
        EntityEvent::EvUseItem14 => "EV_USE_ITEM14",
        EntityEvent::EvUseItem15 => "EV_USE_ITEM15",
        EntityEvent::EvItemRespawn => "EV_ITEM_RESPAWN",
        EntityEvent::EvItemPop => "EV_ITEM_POP",
        EntityEvent::EvPlayerTeleportIn => "EV_PLAYER_TELEPORT_IN",
        EntityEvent::EvPlayerTeleportOut => "EV_PLAYER_TELEPORT_OUT",
        EntityEvent::EvGrenadeBounce => "EV_GRENADE_BOUNCE",
        EntityEvent::EvGeneralSound => "EV_GENERAL_SOUND",
        EntityEvent::EvGlobalSound => "EV_GLOBAL_SOUND",
        EntityEvent::EvGlobalTeamSound => "EV_GLOBAL_TEAM_SOUND",
        EntityEvent::EvBulletHitFlesh => "EV_BULLET_HIT_FLESH",
        EntityEvent::EvBulletHitWall => "EV_BULLET_HIT_WALL",
        EntityEvent::EvMissileHit => "EV_MISSILE_HIT",
        EntityEvent::EvMissileMiss => "EV_MISSILE_MISS",
        EntityEvent::EvMissileMissMetal => "EV_MISSILE_MISS_METAL",
        EntityEvent::EvRailtrail => "EV_RAILTRAIL",
        EntityEvent::EvShotgun => "EV_SHOTGUN",
        EntityEvent::EvBullet => "EV_BULLET",
        EntityEvent::EvPain => "EV_PAIN",
        EntityEvent::EvDeath1 => "EV_DEATH1",
        EntityEvent::EvDeath2 => "EV_DEATH2",
        EntityEvent::EvDeath3 => "EV_DEATH3",
        EntityEvent::EvObituary => "EV_OBITUARY",
        EntityEvent::EvPowerupQuad => "EV_POWERUP_QUAD",
        EntityEvent::EvPowerupBattlesuit => "EV_POWERUP_BATTLESUIT",
        EntityEvent::EvPowerupRegen => "EV_POWERUP_REGEN",
        EntityEvent::EvGibPlayer => "EV_GIB_PLAYER",
        EntityEvent::EvScoreplum => "EV_SCOREPLUM",
        EntityEvent::EvProximityMineStick => "EV_PROXIMITY_MINE_STICK",
        EntityEvent::EvProximityMineTrigger => "EV_PROXIMITY_MINE_TRIGGER",
        EntityEvent::EvKamikaze => "EV_KAMIKAZE",
        EntityEvent::EvObeliskexplode => "EV_OBELISKEXPLODE",
        EntityEvent::EvObeliskpain => "EV_OBELISKPAIN",
        EntityEvent::EvInvulImpact => "EV_INVUL_IMPACT",
        EntityEvent::EvJuiced => "EV_JUICED",
        EntityEvent::EvLightningbolt => "EV_LIGHTNINGBOLT",
        EntityEvent::EvDebugLine => "EV_DEBUG_LINE",
        EntityEvent::EvStoploopingsound => "EV_STOPLOOPINGSOUND",
        EntityEvent::EvTaunt => "EV_TAUNT",
        EntityEvent::EvTauntYes => "EV_TAUNT_YES",
        EntityEvent::EvTauntNo => "EV_TAUNT_NO",
        EntityEvent::EvTauntFollowme => "EV_TAUNT_FOLLOWME",
        EntityEvent::EvTauntGetflag => "EV_TAUNT_GETFLAG",
        EntityEvent::EvTauntGuardbase => "EV_TAUNT_GUARDBASE",
        EntityEvent::EvTauntPatrol => "EV_TAUNT_PATROL",
    })
}

/// Client entity events (`ClientEventRuntime`).
pub struct ClientEventRuntime<H> {
    /// Host services.
    pub host: H,
}

impl<H: ClientEventHost> ClientEventRuntime<H> {
    /// New runtime.
    #[must_use]
    pub fn new(host: H) -> Self {
        Self { host }
    }

    fn check_product(&self, state: &ClientGameState) -> PresentResult<()> {
        if state.product != self.host.product() {
            return Err(state_msg("Event host product differs from cgame state"));
        }
        Ok(())
    }

    fn snapshot<'a>(&self, state: &'a ClientGameState) -> PresentResult<&'a Snapshot> {
        state
            .snap
            .as_ref()
            .ok_or_else(|| state_msg("CG event requires cg.snap"))
    }

    fn center_y(&self, state: &ClientGameState) -> i32 {
        match state.product {
            Product::Baseq3 => 143,
            Product::Missionpack => 144,
        }
    }

    /// Pain event (`painEvent`).
    pub fn pain_event(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        entity_ref: EventEntityRef,
        health: i32,
    ) -> PresentResult<()> {
        self.check_product(state)?;
        let _ = static_state;
        let time = state.time;
        let entity = event_entity_mut(state, entity_ref)?;
        if time.wrapping_sub(entity.player.pain_time) < 500 {
            return Ok(());
        }
        let level = if health < 25 {
            25
        } else if health < 50 {
            50
        } else if health < 75 {
            75
        } else {
            100
        };
        let number = entity.current_state.number;
        let sound = self.host.custom_sound(number, &format!("*pain{level}_1.wav"));
        self.host.start_sound(None, number, EV_VOICE, sound);
        let entity = event_entity_mut(state, entity_ref)?;
        entity.player.pain_time = time;
        entity.player.pain_direction = !entity.player.pain_direction;
        Ok(())
    }

    fn item_pickup(&mut self, state: &mut ClientGameState, index: i32) -> PresentResult<()> {
        state.item_pickup = index;
        state.item_pickup_time = state.time;
        state.item_pickup_blend_time = state.time;
        let item = *item_at(state.product, index)?;
        if matches!(item.kind, ItemKind::Weapon(_))
            && self.host.options().autoswitch
            && item.tag() != Weapon::WpMachinegun as i32
        {
            state.weapon_select_time = state.time;
            state.weapon_select = item.tag();
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn use_item(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        entity_ref: EventEntityRef,
    ) -> PresentResult<()> {
        let es = event_entity(state, entity_ref)?.current_state.clone();
        let mut item = (es.event & !EV_EVENT_BITS) - EntityEvent::EvUseItem0 as i32;
        if item < 0 || item > Holdable::HiNumHoldable as i32 {
            item = 0;
        }
        if es.number == self.snapshot(state)?.player_state.client_num {
            let text = if item == 0 {
                "No item to use".to_owned()
            } else {
                format!(
                    "Use {}",
                    find_item_for_holdable(state.product, holdable_from_tag(item)?)?
                        .pickup_name
                        .unwrap_or("")
                )
            };
            let y = self.center_y(state);
            self.host.center_print(&text, y, 16);
        }
        if item == Holdable::HiTeleporter as i32 {
            return Ok(());
        }
        if item == Holdable::HiMedkit as i32 {
            if es.client_num >= 0 && es.client_num < MAX_CLIENTS as i32 {
                let time = state.time;
                self.host.set_medkit_usage_time(static_state, es.client_num, time)?;
            }
            let sound = self.host.media().sounds.medkit_sound;
            self.host.start_sound(None, es.number, EV_BODY, sound);
            return Ok(());
        }
        if self.host.product() == Product::Missionpack {
            if item == Holdable::HiKamikaze as i32 || item == Holdable::HiPortal as i32 {
                return Ok(());
            }
            if item == Holdable::HiInvulnerability as i32 {
                let sound = self
                    .host
                    .mission_sounds()
                    .and_then(|sounds| sounds.use_invulnerability_sound);
                self.host.start_sound(None, es.number, EV_BODY, sound);
                return Ok(());
            }
        }
        let sound = self.host.media().sounds.use_nothing_sound;
        self.host.start_sound(None, es.number, EV_BODY, sound);
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn obituary(
        &mut self,
        state: &mut ClientGameState,
        static_state: &ClientGameStaticState,
        es: &EntityState,
    ) -> PresentResult<()> {
        let target = es.other_entity_num;
        let modification = es.event_parm;
        let mut attacker = es.other_entity_num2;
        if target < 0 || target >= MAX_CLIENTS as i32 {
            return Err(drop_msg("CG_Obituary: target out of range"));
        }
        let info = self.host.client_info_brief(static_state, target)?;
        let mut attacker_name = if attacker < 0 || attacker >= MAX_CLIENTS as i32 {
            attacker = ENTITYNUM_WORLD;
            None
        } else {
            self.host.player_name(attacker)
        };
        let Some(name) = self.host.player_name(target) else {
            return Ok(());
        };
        let target_name = format!("{}^7", name.chars().take(29).collect::<String>());
        let mut message: Option<String> = match modification {
            20 => Some("suicides".to_owned()),
            19 => Some("cratered".to_owned()),
            17 => Some("was squished".to_owned()),
            14 => Some("sank like a rock".to_owned()),
            15 => Some("melted".to_owned()),
            16 => Some("does a back flip into the lava".to_owned()),
            21 => Some("saw the light".to_owned()),
            22 => Some("was in the wrong place".to_owned()),
            _ => None,
        };
        if attacker == target {
            let ourselves = match info.gender {
                PlayerGender::Female => "herself",
                PlayerGender::Neuter => "itself",
                _ => "himself",
            };
            let possessive = match info.gender {
                PlayerGender::Female => "her",
                PlayerGender::Neuter => "its",
                _ => "his",
            };
            let mission = state.product == Product::Missionpack;
            message = Some(if mission && modification == 26 {
                "goes out with a bang".to_owned()
            } else if modification == 5 {
                format!("tripped on {possessive} own grenade")
            } else if modification == 7 {
                format!("blew {ourselves} up")
            } else if modification == 9 {
                format!("melted {ourselves}")
            } else if modification == 13 {
                "should have used a smaller gun".to_owned()
            } else if mission && modification == 25 {
                let found = if info.gender == PlayerGender::Neuter {
                    "it's"
                } else {
                    possessive
                };
                format!("found {found} prox mine")
            } else {
                format!("killed {ourselves}")
            });
        }
        if let Some(message) = message {
            self.host.print(&format!("{target_name} {message}.\n"));
            return Ok(());
        }
        let ps = self.snapshot(state)?.player_state.clone();
        if attacker == ps.client_num {
            let mut text = format!("You fragged {target_name}");
            if (self.host.options().game_type as i32) < GameType::GtTeam as i32 {
                text += &game_format_bounded(
                    "\n%s place with %i",
                    &[
                        GameFormatArgument::from(place_string(
                            ps.persistant.get(PersistentIndex::PersRank as usize).wrapping_add(1),
                        )),
                        GameFormatArgument::from(ps.persistant.get(PersistentIndex::PersScore as usize)),
                    ],
                    16_384,
                );
            }
            let mission = state.product == Product::Missionpack;
            if !mission || !(self.host.options().single_player_active && self.host.options().camera_orbit) {
                let y = self.center_y(state);
                self.host.center_print(&text, y, 16);
            }
        }
        if attacker_name.is_none() {
            attacker = ENTITYNUM_WORLD;
            attacker_name = Some("noname".to_owned());
        } else {
            let name = attacker_name.expect("checked attacker name");
            let trimmed = format!("{}^7", name.chars().take(29).collect::<String>());
            if target == self.snapshot(state)?.player_state.client_num {
                state.killer_name = trimmed.clone();
            }
            attacker_name = Some(trimmed);
        }
        let mut suffix = "";
        if attacker != ENTITYNUM_WORLD {
            let mission = state.product == Product::Missionpack;
            message = Some(match modification {
                2 => "was pummeled by".to_owned(),
                3 => "was machinegunned by".to_owned(),
                1 => "was gunned down by".to_owned(),
                4 => {
                    suffix = "'s grenade";
                    "ate".to_owned()
                }
                5 => {
                    suffix = "'s shrapnel";
                    "was shredded by".to_owned()
                }
                6 => {
                    suffix = "'s rocket";
                    "ate".to_owned()
                }
                7 => {
                    suffix = "'s rocket";
                    "almost dodged".to_owned()
                }
                8 | 9 => {
                    suffix = "'s plasmagun";
                    "was melted by".to_owned()
                }
                10 => "was railed by".to_owned(),
                11 => "was electrocuted by".to_owned(),
                12 | 13 => {
                    suffix = "'s BFG";
                    "was blasted by".to_owned()
                }
                18 => {
                    suffix = "'s personal space";
                    "tried to invade".to_owned()
                }
                _ => {
                    if modification == (if mission { 28 } else { 23 }) {
                        "was caught by".to_owned()
                    } else if mission && modification == 23 {
                        "was nailed by".to_owned()
                    } else if mission && modification == 24 {
                        suffix = "'s Chaingun";
                        "got lead poisoning from".to_owned()
                    } else if mission && modification == 25 {
                        suffix = "'s Prox Mine";
                        "was too close to".to_owned()
                    } else if mission && modification == 26 {
                        suffix = "'s Kamikaze blast";
                        "falls to".to_owned()
                    } else if mission && modification == 27 {
                        "was juiced by".to_owned()
                    } else {
                        "was killed by".to_owned()
                    }
                }
            });
            let attacker_name = attacker_name.expect("checked attacker name");
            self.host.print(&format!(
                "{target_name} {}{attacker_name}{suffix}\n",
                message.expect("obituary message")
            ));
            return Ok(());
        }
        self.host.print(&format!("{target_name} died.\n"));
        Ok(())
    }

    fn team_sound(
        &mut self,
        state: &mut ClientGameState,
        static_state: &ClientGameStaticState,
        event: i32,
    ) -> PresentResult<()> {
        let team = self.host.client_info_brief(static_state, state.client_num)?.team;
        match event {
            0 | 1 => {
                let expected = if event == 0 { Team::TeamRed } else { Team::TeamBlue };
                let sounds = self.host.media().sounds;
                self.host.add_buffered_sound(if team == expected {
                    sounds.capture_your_team_sound
                } else {
                    sounds.capture_opponent_sound
                });
            }
            2 | 3 => {
                let expected = if event == 2 { Team::TeamRed } else { Team::TeamBlue };
                let sounds = self.host.media().sounds;
                self.host.add_buffered_sound(if team == expected {
                    sounds.return_your_team_sound
                } else {
                    sounds.return_opponent_sound
                });
                self.host.add_buffered_sound(if event == 2 {
                    sounds.blue_flag_returned_sound
                } else {
                    sounds.red_flag_returned_sound
                });
            }
            4 | 5 => {
                let ps = self.snapshot(state)?.player_state.clone();
                let held = if event == 4 {
                    Powerup::PwBlueflag
                } else {
                    Powerup::PwRedflag
                };
                if ps.powerups.get(held as usize) != 0 || ps.powerups.get(Powerup::PwNeutralflag as usize) != 0 {
                    return Ok(());
                }
                let threatened = if event == 4 { Team::TeamBlue } else { Team::TeamRed };
                if team != Team::TeamRed && team != Team::TeamBlue {
                    return Ok(());
                }
                if self.host.product() == Product::Missionpack && self.host.options().game_type == GameType::Gt1fctf {
                    let sounds = self.host.mission_sounds().copied().unwrap_or_default();
                    self.host.add_buffered_sound(if team == threatened {
                        sounds.your_team_took_the_flag_sound
                    } else {
                        sounds.enemy_took_the_flag_sound
                    });
                } else {
                    let sounds = self.host.media().sounds;
                    self.host.add_buffered_sound(if team == threatened {
                        sounds.enemy_took_your_flag_sound
                    } else {
                        sounds.your_team_took_enemy_flag_sound
                    });
                }
            }
            6 | 7 => {
                let expected = if event == 6 { Team::TeamRed } else { Team::TeamBlue };
                if team == expected {
                    let sound = self.host.media().sounds.your_base_is_under_attack_sound;
                    self.host.add_buffered_sound(sound);
                }
            }
            8 => {
                let sound = self.host.media().sounds.red_scored_sound;
                self.host.add_buffered_sound(sound);
            }
            9 => {
                let sound = self.host.media().sounds.blue_scored_sound;
                self.host.add_buffered_sound(sound);
            }
            10 => {
                let sound = self.host.media().sounds.red_leads_sound;
                self.host.add_buffered_sound(sound);
            }
            11 => {
                let sound = self.host.media().sounds.blue_leads_sound;
                self.host.add_buffered_sound(sound);
            }
            12 => {
                let sound = self.host.media().sounds.teams_tied_sound;
                self.host.add_buffered_sound(sound);
            }
            13 if self.host.product() == Product::Missionpack => {
                let sound = self.host.mission_sounds().and_then(|sounds| sounds.kamikaze_far_sound);
                self.host.start_local_sound(sound, EV_ANNOUNCER);
            }
            _ => {}
        }
        Ok(())
    }

    /// Dispatch an entity event through the recipe hook.
    pub fn entity_event(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        entity_ref: EventEntityRef,
        position: Vec3,
    ) -> PresentResult<()> {
        self.check_product(state)?;
        if self.host.pre_present_event(state, entity_ref, position) {
            self.source_entity_event(state, static_state, entity_ref, position)?;
            self.host.post_present_event(state, entity_ref, position);
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn source_entity_event(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        entity_ref: EventEntityRef,
        position: Vec3,
    ) -> PresentResult<()> {
        let es = event_entity(state, entity_ref)?.current_state.clone();
        let event = es.event & !EV_EVENT_BITS;
        if self.host.options().debug_events {
            let text = game_format_bounded(
                "ent:%3i  event:%3i ",
                &[GameFormatArgument::from(es.number), GameFormatArgument::from(event)],
                16_384,
            );
            self.host.print(&text);
        }
        if event == 0 {
            if self.host.options().debug_events {
                self.host.print("ZEROEVENT\n");
            }
            return Ok(());
        }
        let client_num = if es.client_num < 0 || es.client_num >= MAX_CLIENTS as i32 {
            0
        } else {
            es.client_num
        };
        let info = self.host.client_info_brief(static_state, client_num)?;
        let mission = self.host.product() == Product::Missionpack;
        let mission_only = (event >= EntityEvent::EvProximityMineStick as i32
            && event <= EntityEvent::EvLightningbolt as i32)
            || event >= EntityEvent::EvTauntYes as i32;
        let known = entity_event_name(event);
        if known.is_none()
            || event == EntityEvent::EvUseItem15 as i32
            || event == EntityEvent::EvBullet as i32
            || (mission_only && !mission)
        {
            if self.host.options().debug_events {
                self.host.print("UNKNOWN\n");
            }
            return Err(drop_msg(format!("Unknown event: {event}")));
        }
        if self.host.options().debug_events {
            let label = if event >= EntityEvent::EvStep4 as i32 && event <= EntityEvent::EvStep16 as i32 {
                "EV_STEP"
            } else if event >= EntityEvent::EvDeath1 as i32 && event <= EntityEvent::EvDeath3 as i32 {
                "EV_DEATHx"
            } else {
                known.expect("checked event name")
            };
            self.host.print(&format!("{label}\n"));
        }
        if event >= EntityEvent::EvUseItem0 as i32 && event <= EntityEvent::EvUseItem14 as i32 {
            self.use_item(state, static_state, entity_ref)?;
            return Ok(());
        }
        let event_enum = entity_event_from_i32(event).expect("checked known event");
        match event_enum {
            EntityEvent::EvFootstep
            | EntityEvent::EvFootstepMetal
            | EntityEvent::EvFootsplash
            | EntityEvent::EvFootwade
            | EntityEvent::EvSwim => {
                if self.host.options().footsteps {
                    let pick = (self.host.rand_int() & 3) as usize;
                    let bank;
                    let sounds = if event_enum == EntityEvent::EvFootstep {
                        bank = *self.host.media().footsteps.bank(info.footsteps);
                        &bank
                    } else if event_enum == EntityEvent::EvFootstepMetal {
                        &self.host.media().footsteps.metal
                    } else {
                        &self.host.media().footsteps.splash
                    };
                    let sound = sounds[pick];
                    self.host.start_sound(None, es.number, EV_BODY, sound);
                }
            }
            EntityEvent::EvFallShort | EntityEvent::EvFallMedium | EntityEvent::EvFallFar => {
                if event_enum == EntityEvent::EvFallShort {
                    let sound = self.host.media().sounds.land_sound;
                    self.host.start_sound(None, es.number, EV_AUTO, sound);
                } else {
                    let (channel, name) = if event_enum == EntityEvent::EvFallMedium {
                        (EV_VOICE, "*pain100_1.wav")
                    } else {
                        (EV_AUTO, "*fall1.wav")
                    };
                    let sound = self.host.custom_sound(es.number, name);
                    self.host.start_sound(None, es.number, channel, sound);
                }
                if event_enum == EntityEvent::EvFallFar {
                    event_entity_mut(state, entity_ref)?.player.pain_time = state.time;
                }
                if client_num == state.predicted_player_state.client_num {
                    state.land_change = -8.0 * (event - EntityEvent::EvFallShort as i32 + 1) as f32;
                    state.land_time = state.time;
                }
            }
            EntityEvent::EvStep4 | EntityEvent::EvStep8 | EntityEvent::EvStep12 | EntityEvent::EvStep16 => {
                if client_num == state.predicted_player_state.client_num
                    && !self.host.options().demo_playback
                    && (self.snapshot(state)?.player_state.pm_flags & (MoveFlags::Follow as i32)) == 0
                    && !self.host.options().no_predict
                    && !self.host.options().synchronous_clients
                {
                    let delta = state.time.wrapping_sub(state.step_time);
                    let old_step = if delta < 200 {
                        state.step_change * (200 - delta) as f32 / 200.0
                    } else {
                        0.0
                    };
                    state.step_change = (old_step + 4.0 * (event - EntityEvent::EvStep4 as i32 + 1) as f32).min(32.0);
                    state.step_time = state.time;
                }
            }
            EntityEvent::EvJumpPad => {
                let shader = self.host.media().smoke_puff_shader.clone();
                self.host.smoke_puff(
                    state,
                    &SmokePuffDesc {
                        origin: event_entity(state, entity_ref)?.lerp_origin,
                        velocity: vec3(0.0, 0.0, 1.0),
                        radius: 32.0,
                        color: vec4(1.0, 1.0, 1.0, 0.33),
                        duration: 1000,
                        start_time: state.time,
                        fade_in_time: 0,
                        flags: 1,
                        shader,
                    },
                    None,
                );
                let sound = self.host.media().sounds.jump_pad_sound;
                let origin = event_entity(state, entity_ref)?.lerp_origin;
                self.host.start_sound(Some(origin), -1, EV_VOICE, sound);
                let sound = self.host.custom_sound(es.number, "*jump1.wav");
                self.host.start_sound(None, es.number, EV_VOICE, sound);
            }
            EntityEvent::EvJump => {
                let sound = self.host.custom_sound(es.number, "*jump1.wav");
                self.host.start_sound(None, es.number, EV_VOICE, sound);
            }
            EntityEvent::EvTaunt => {
                let sound = self.host.custom_sound(es.number, "*taunt.wav");
                self.host.start_sound(None, es.number, EV_VOICE, sound);
            }
            EntityEvent::EvTauntYes
            | EntityEvent::EvTauntNo
            | EntityEvent::EvTauntFollowme
            | EntityEvent::EvTauntGetflag
            | EntityEvent::EvTauntGuardbase
            | EntityEvent::EvTauntPatrol => {
                if mission {
                    let command = match event_enum {
                        EntityEvent::EvTauntYes => "yes",
                        EntityEvent::EvTauntNo => "no",
                        EntityEvent::EvTauntFollowme => "followme",
                        EntityEvent::EvTauntGetflag => "ongetflag",
                        EntityEvent::EvTauntGuardbase => "ondefense",
                        _ => "onpatrol",
                    };
                    self.host
                        .voice_chat_local(state, static_state, 1, false, es.number, 53, command)?;
                }
            }
            EntityEvent::EvWaterTouch => {
                let sound = self.host.media().sounds.watr_in_sound;
                self.host.start_sound(None, es.number, EV_AUTO, sound);
            }
            EntityEvent::EvWaterLeave => {
                let sound = self.host.media().sounds.watr_out_sound;
                self.host.start_sound(None, es.number, EV_AUTO, sound);
            }
            EntityEvent::EvWaterUnder => {
                let sound = self.host.media().sounds.watr_un_sound;
                self.host.start_sound(None, es.number, EV_AUTO, sound);
            }
            EntityEvent::EvWaterClear => {
                let sound = self.host.custom_sound(es.number, "*gasp.wav");
                self.host.start_sound(None, es.number, EV_AUTO, sound);
            }
            EntityEvent::EvItemPickup | EntityEvent::EvGlobalItemPickup => {
                let index = es.event_parm;
                if index >= 1 && index < item_list(state.product).len() as i32 {
                    let item = *item_at(state.product, index)?;
                    if event_enum == EntityEvent::EvGlobalItemPickup {
                        if let Some(path) = item.pickup_sound {
                            let sound = self.host.register_sound(Some(path), false);
                            let client = self.snapshot(state)?.player_state.client_num;
                            self.host.start_sound(None, client, EV_AUTO, sound);
                        }
                    } else if matches!(item.kind, ItemKind::Powerup(_) | ItemKind::Team(_)) {
                        let sound = self.host.media().sounds.n_health_sound;
                        self.host.start_sound(None, es.number, EV_AUTO, sound);
                    } else if matches!(item.kind, ItemKind::PersistantPowerup(_)) {
                        if mission {
                            let sounds = self.host.mission_sounds().copied().unwrap_or_default();
                            let sound = if item.tag() == Powerup::PwScout as i32 {
                                sounds.scout_sound
                            } else if item.tag() == Powerup::PwGuard as i32 {
                                sounds.guard_sound
                            } else if item.tag() == Powerup::PwDoubler as i32 {
                                sounds.doubler_sound
                            } else if item.tag() == Powerup::PwAmmoregen as i32 {
                                sounds.ammoregen_sound
                            } else {
                                None
                            };
                            if item.tag() == Powerup::PwScout as i32
                                || item.tag() == Powerup::PwGuard as i32
                                || item.tag() == Powerup::PwDoubler as i32
                                || item.tag() == Powerup::PwAmmoregen as i32
                            {
                                self.host.start_sound(None, es.number, EV_AUTO, sound);
                            }
                        }
                    } else {
                        let sound = self.host.register_sound(item.pickup_sound, false);
                        self.host.start_sound(None, es.number, EV_AUTO, sound);
                    }
                    if es.number == self.snapshot(state)?.player_state.client_num {
                        self.item_pickup(state, index)?;
                    }
                }
            }
            EntityEvent::EvNoammo => {
                if es.number == self.snapshot(state)?.player_state.client_num {
                    self.host.out_of_ammo_change(state);
                }
            }
            EntityEvent::EvChangeWeapon => {
                let sound = self.host.media().sounds.select_sound;
                self.host.start_sound(None, es.number, EV_AUTO, sound);
            }
            EntityEvent::EvFireWeapon => {
                self.host.fire_weapon(state, entity_ref);
            }
            EntityEvent::EvPlayerTeleportIn | EntityEvent::EvPlayerTeleportOut => {
                let sounds = self.host.media().sounds;
                self.host.start_sound(
                    None,
                    es.number,
                    EV_AUTO,
                    if event_enum == EntityEvent::EvPlayerTeleportIn {
                        sounds.tele_in_sound
                    } else {
                        sounds.tele_out_sound
                    },
                );
                self.host.spawn_effect(state, position);
            }
            EntityEvent::EvItemPop => {
                let sound = self.host.media().sounds.respawn_sound;
                self.host.start_sound(None, es.number, EV_AUTO, sound);
            }
            EntityEvent::EvItemRespawn => {
                let time = state.time;
                event_entity_mut(state, entity_ref)?.misc_time = time;
                let sound = self.host.media().sounds.respawn_sound;
                self.host.start_sound(None, es.number, EV_AUTO, sound);
            }
            EntityEvent::EvGrenadeBounce => {
                let sounds = self.host.media().sounds;
                let sound = if (self.host.rand_int() & 1) != 0 {
                    sounds.hgrenb1a_sound
                } else {
                    sounds.hgrenb2a_sound
                };
                self.host.start_sound(None, es.number, EV_AUTO, sound);
            }
            EntityEvent::EvProximityMineStick => {
                if mission {
                    let sounds = self.host.mission_sounds().copied().unwrap_or_default();
                    let sound = if (es.event_parm & 64) != 0 {
                        sounds.wstbimpl_sound
                    } else if (es.event_parm & 4096) != 0 {
                        sounds.wstbimpm_sound
                    } else {
                        sounds.wstbimpd_sound
                    };
                    self.host.start_sound(None, es.number, EV_AUTO, sound);
                }
            }
            EntityEvent::EvProximityMineTrigger => {
                if mission {
                    let sound = self.host.mission_sounds().and_then(|sounds| sounds.wstbactv_sound);
                    self.host.start_sound(None, es.number, EV_AUTO, sound);
                }
            }
            EntityEvent::EvKamikaze => {
                if mission {
                    let origin = event_entity(state, entity_ref)?.lerp_origin;
                    self.host.kamikaze_effect(state, origin);
                }
            }
            EntityEvent::EvObeliskexplode => {
                if mission {
                    let origin = event_entity(state, entity_ref)?.lerp_origin;
                    self.host.obelisk_explode(state, origin);
                }
            }
            EntityEvent::EvObeliskpain => {
                if mission {
                    let origin = event_entity(state, entity_ref)?.lerp_origin;
                    self.host.obelisk_pain(state, origin);
                }
            }
            EntityEvent::EvInvulImpact => {
                if mission {
                    let origin = event_entity(state, entity_ref)?.lerp_origin;
                    self.host.invulnerability_impact(state, origin, es.angles);
                }
            }
            EntityEvent::EvJuiced => {
                if mission {
                    let origin = event_entity(state, entity_ref)?.lerp_origin;
                    self.host.invulnerability_juiced(state, origin);
                }
            }
            EntityEvent::EvLightningbolt => {
                if mission {
                    self.host.lightning_bolt_beam(state, es.origin2, es.pos.base);
                }
            }
            EntityEvent::EvScoreplum => {
                let origin = event_entity(state, entity_ref)?.lerp_origin;
                self.host.score_plum(state, es.other_entity_num, origin, es.time);
            }
            EntityEvent::EvMissileHit => {
                self.host.missile_hit_player(
                    state,
                    es.weapon,
                    position,
                    byte_to_direction(es.event_parm),
                    es.other_entity_num,
                );
            }
            EntityEvent::EvMissileMiss | EntityEvent::EvMissileMissMetal => {
                self.host.missile_hit_wall(
                    state,
                    es.weapon,
                    0,
                    position,
                    byte_to_direction(es.event_parm),
                    if event_enum == EntityEvent::EvMissileMiss {
                        ImpactSound::Default
                    } else {
                        ImpactSound::Metal
                    },
                );
            }
            EntityEvent::EvRailtrail => {
                event_entity_mut(state, entity_ref)?.current_state.weapon = Weapon::WpRailgun as i32;
                self.host.rail_trail(state, client_num, es.origin2, es.pos.base);
                if es.event_parm != 255 {
                    self.host.missile_hit_wall(
                        state,
                        Weapon::WpRailgun as i32,
                        es.client_num,
                        position,
                        byte_to_direction(es.event_parm),
                        ImpactSound::Default,
                    );
                }
            }
            EntityEvent::EvBulletHitWall => {
                self.host.bullet(
                    state,
                    es.pos.base,
                    es.other_entity_num,
                    BulletTarget::Wall {
                        normal: byte_to_direction(es.event_parm),
                    },
                );
            }
            EntityEvent::EvBulletHitFlesh => {
                self.host.bullet(
                    state,
                    es.pos.base,
                    es.other_entity_num,
                    BulletTarget::Flesh {
                        entity_num: es.event_parm,
                    },
                );
            }
            EntityEvent::EvShotgun => {
                self.host.shotgun_fire(state, entity_ref);
            }
            EntityEvent::EvGeneralSound | EntityEvent::EvGlobalSound => {
                let sounds = self.host.media().game_sounds.clone();
                let sound = sounds
                    .get(es.event_parm as usize)
                    .copied()
                    .ok_or_else(|| range_msg(format!("Unregistered game sound index {}", es.event_parm)))?;
                if es.event_parm < 0 {
                    return Err(range_msg(format!("Unregistered game sound index {}", es.event_parm)));
                }
                let client = self.snapshot(state)?.player_state.client_num;
                let sound = match sound {
                    Some(sound) => Some(sound),
                    None => {
                        let path = self.host.sound_config_string(es.event_parm);
                        self.host.custom_sound(es.number, &path)
                    }
                };
                if event_enum == EntityEvent::EvGeneralSound {
                    self.host.start_sound(None, es.number, EV_VOICE, sound);
                } else {
                    self.host.start_sound(None, client, EV_AUTO, sound);
                }
            }
            EntityEvent::EvGlobalTeamSound => {
                self.team_sound(state, static_state, es.event_parm)?;
            }
            EntityEvent::EvPain => {
                if es.number != self.snapshot(state)?.player_state.client_num {
                    self.pain_event(state, static_state, entity_ref, es.event_parm)?;
                }
            }
            EntityEvent::EvDeath1 | EntityEvent::EvDeath2 | EntityEvent::EvDeath3 => {
                let sound = self.host.custom_sound(
                    es.number,
                    &format!("*death{}.wav", event - EntityEvent::EvDeath1 as i32 + 1),
                );
                self.host.start_sound(None, es.number, EV_VOICE, sound);
            }
            EntityEvent::EvObituary => {
                self.obituary(state, static_state, &es)?;
            }
            EntityEvent::EvPowerupQuad | EntityEvent::EvPowerupBattlesuit | EntityEvent::EvPowerupRegen => {
                if es.number == self.snapshot(state)?.player_state.client_num {
                    state.powerup_active = if event_enum == EntityEvent::EvPowerupQuad {
                        Powerup::PwQuad as i32
                    } else if event_enum == EntityEvent::EvPowerupBattlesuit {
                        Powerup::PwBattlesuit as i32
                    } else {
                        Powerup::PwRegen as i32
                    };
                    state.powerup_time = state.time;
                }
                let sounds = self.host.media().sounds;
                self.host.start_sound(
                    None,
                    es.number,
                    EV_ITEM,
                    if event_enum == EntityEvent::EvPowerupQuad {
                        sounds.quad_sound
                    } else if event_enum == EntityEvent::EvPowerupBattlesuit {
                        sounds.protect_sound
                    } else {
                        sounds.regen_sound
                    },
                );
            }
            EntityEvent::EvGibPlayer => {
                if (es.e_flags & 512) == 0 {
                    let sound = self.host.media().sounds.gib_sound;
                    self.host.start_sound(None, es.number, EV_BODY, sound);
                }
                let origin = event_entity(state, entity_ref)?.lerp_origin;
                self.host.gib_player(state, origin);
            }
            EntityEvent::EvStoploopingsound => {
                self.host.stop_looping_sound(es.number);
                event_entity_mut(state, entity_ref)?.current_state.loop_sound = 0;
            }
            EntityEvent::EvDebugLine => {
                self.host.beam(state, entity_ref);
            }
            EntityEvent::EvNone
            | EntityEvent::EvUseItem0
            | EntityEvent::EvUseItem1
            | EntityEvent::EvUseItem2
            | EntityEvent::EvUseItem3
            | EntityEvent::EvUseItem4
            | EntityEvent::EvUseItem5
            | EntityEvent::EvUseItem6
            | EntityEvent::EvUseItem7
            | EntityEvent::EvUseItem8
            | EntityEvent::EvUseItem9
            | EntityEvent::EvUseItem10
            | EntityEvent::EvUseItem11
            | EntityEvent::EvUseItem12
            | EntityEvent::EvUseItem13
            | EntityEvent::EvUseItem14
            | EntityEvent::EvUseItem15
            | EntityEvent::EvBullet => {
                return Err(drop_msg(format!("Unknown event: {event}")));
            }
        }
        Ok(())
    }

    /// Check an entity's events (`checkEvents`).
    pub fn check_events(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        entity_number: usize,
    ) -> PresentResult<()> {
        self.check_product(state)?;
        let number = i32::try_from(entity_number).map_err(|_| range_msg("Entity number outside int32"))?;
        let entity = state.entity_at_mut(number)?;
        if entity.current_state.e_type > EntityType::EtEvents as i32 {
            if entity.previous_event != 0 {
                return Ok(());
            }
            if (entity.current_state.e_flags & 16) != 0 {
                let other = entity.current_state.other_entity_num;
                entity.current_state.number = other;
            }
            entity.previous_event = 1;
            let event = entity.current_state.e_type - EntityType::EtEvents as i32;
            entity.current_state.event = event;
        } else {
            if entity.current_state.event == entity.previous_event {
                return Ok(());
            }
            let event = entity.current_state.event;
            entity.previous_event = event;
            if (event & !EV_EVENT_BITS) == 0 {
                return Ok(());
            }
        }
        let server_time = self.snapshot(state)?.server_time;
        let entity = state.entity_at_mut(number)?;
        entity.lerp_origin = evaluate_trajectory(&entity.current_state.pos, server_time);
        self.host.set_entity_sound_position(state, entity_number);
        let position = state.entity_at(number)?.lerp_origin;
        self.entity_event(state, static_state, EventEntityRef::Entity(entity_number), position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_string_ranks() {
        assert_eq!(place_string(1), "^41st^7");
        assert_eq!(place_string(2), "^12nd^7");
        assert_eq!(place_string(3), "^33rd^7");
        assert_eq!(place_string(4), "4th");
        assert_eq!(place_string(11), "11th");
        assert_eq!(place_string(21), "21st");
        assert_eq!(place_string(0x4000 | 1), "Tied for ^41st^7");
    }
}
