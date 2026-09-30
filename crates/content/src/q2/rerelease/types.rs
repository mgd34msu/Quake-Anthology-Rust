//! Q2 rerelease shared types (`src/content/q2/rerelease/types.ts`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use qa_core::identity::ActorId;
use qa_core::math::{Vec3, Vec4};

use crate::q2::base::player::types::Q2PlayerHand;
use crate::q2::foundation::host::Q2GameServices;
use crate::q2::support::misc::{DebugLine, WorldTextInput};

use super::campaign::Q2RereleaseLevelEntry;

/// Rerelease fog (`Q2Fog`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Q2Fog {
    /// Density.
    pub density: f64,
    /// Color.
    pub color: Vec3,
    /// Sky factor.
    pub sky_factor: f64,
}

/// Rerelease height fog (`Q2HeightFog`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Q2HeightFog {
    /// Start color.
    pub start_color: Vec3,
    /// Start distance.
    pub start_distance: f64,
    /// End color.
    pub end_color: Vec3,
    /// End distance.
    pub end_distance: f64,
    /// Falloff.
    pub falloff: f64,
    /// Density.
    pub density: f64,
}

/// Rerelease fog state (`Q2FogState`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Q2FogState {
    /// Fog.
    pub fog: Q2Fog,
    /// Height fog.
    pub height_fog: Q2HeightFog,
}

/// Create empty rerelease fog (`createQ2Fog`).
pub fn create_q2_fog() -> Q2FogState {
    Q2FogState::default()
}

/// Coop respawn state (`Q2CoopRespawnState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Q2CoopRespawnState {
    /// No respawn pending.
    #[default]
    None,
    /// In combat.
    InCombat,
    /// Bad area.
    BadArea,
    /// Blocked.
    Blocked,
    /// Waiting.
    Waiting,
    /// No lives left.
    NoLives,
}

/// Pending landmark carry without its player (`Omit<Q2LandmarkCarry, "player">`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Q2PendingLandmark {
    /// Landmark name.
    pub name: String,
    /// Relative origin.
    pub relative_origin: Vec3,
    /// Relative velocity.
    pub relative_velocity: Vec3,
    /// Relative view angles.
    pub relative_view_angles: Vec3,
}

/// Rerelease per-player state (`Q2RereleasePlayerState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleasePlayerState {
    /// Whether spawned.
    pub spawned: bool,
    /// Primary help change counter.
    pub game_help1_changed: i32,
    /// Secondary help change counter.
    pub game_help2_changed: i32,
    /// Help change counter.
    pub help_changed: i32,
    /// Help time.
    pub help_time: f64,
    /// Invisibility expiry.
    pub invisibility_until: f64,
    /// Invisibility fade expiry.
    pub invisibility_fade_until: f64,
    /// Slime damage debounce.
    pub slime_debounce: f64,
    /// Animation time.
    pub animation_time: f64,
    /// Flash time.
    pub flash_time: f64,
    /// Flash count.
    pub flashes: i32,
    /// Last damage time.
    pub last_damage_until: f64,
    /// Last firing time.
    pub last_firing_until: f64,
    /// Coop lives.
    pub lives: i32,
    /// Coop respawn state.
    pub coop_respawn_state: Q2CoopRespawnState,
    /// Whether the flashlight is on.
    pub flashlight: bool,
    /// Current fog.
    pub fog: Q2FogState,
    /// Wanted fog.
    pub wanted_fog: Q2FogState,
    /// Fog transition.
    pub fog_transition: f64,
    /// Whether to skip the view bob.
    pub bob_skip: bool,
    /// Auto switch mode 0-3.
    pub auto_switch: u8,
    /// Auto shield selection.
    pub auto_shield: i32,
    /// Player dogtag.
    pub dogtag: String,
    /// Pending impact delta.
    pub impact_delta: f64,
    /// Whether on a ladder.
    pub on_ladder: bool,
    /// Grapple release time.
    pub grapple_released_until: f64,
    /// Whether the grapple is attached.
    pub grapple_attached: bool,
    /// Slow view angles.
    pub slow_view_angles: Vec3,
    /// Quake time.
    pub quake_time: f64,
    /// Wind sound time.
    pub wind_sound_time: f64,
    /// Whether awaiting respawn.
    pub awaiting_respawn: bool,
    /// Respawn timeout.
    pub respawn_timeout: f64,
    /// Pending landmark carry.
    pub pending_landmark: Option<Q2PendingLandmark>,
    /// Help location.
    pub help_location: Vec3,
    /// Help image.
    pub help_image: String,
    /// Help path points.
    pub help_points: Vec<Vec3>,
    /// Help path index.
    pub help_index: i32,
    /// Help draw time.
    pub help_draw_time: f64,
    /// Help marker expiry.
    pub help_marker_until: f64,
    /// Seat number.
    pub seat: i32,
    /// Social id.
    pub social_id: String,
}

impl Q2RereleasePlayerState {
    /// Admit a rerelease player state for a seat and social id.
    pub fn new(seat: i32, social_id: String) -> Self {
        Self {
            seat,
            social_id,
            ..Self::default()
        }
    }
}

impl Default for Q2RereleasePlayerState {
    fn default() -> Self {
        Self {
            spawned: false,
            game_help1_changed: 0,
            game_help2_changed: 0,
            help_changed: 0,
            help_time: 0.0,
            invisibility_until: 0.0,
            invisibility_fade_until: 0.0,
            slime_debounce: 0.0,
            animation_time: 0.0,
            flash_time: 0.0,
            flashes: 0,
            last_damage_until: 0.0,
            last_firing_until: 0.0,
            lives: 0,
            coop_respawn_state: Q2CoopRespawnState::None,
            flashlight: false,
            fog: create_q2_fog(),
            wanted_fog: create_q2_fog(),
            fog_transition: 0.0,
            bob_skip: false,
            auto_switch: 0,
            auto_shield: -1,
            dogtag: String::new(),
            impact_delta: 0.0,
            on_ladder: false,
            grapple_released_until: 0.0,
            grapple_attached: false,
            slow_view_angles: Vec3::default(),
            quake_time: 0.0,
            wind_sound_time: 0.0,
            awaiting_respawn: false,
            respawn_timeout: 0.0,
            pending_landmark: None,
            help_location: Vec3::default(),
            help_image: "friend".to_string(),
            help_points: Vec::new(),
            help_index: 0,
            help_draw_time: 0.0,
            help_marker_until: 0.0,
            seat: 0,
            social_id: String::new(),
        }
    }
}

/// Rerelease options (`Q2RereleaseOptions`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleaseOptions {
    /// Coop squad respawn.
    pub coop_squad_respawn: bool,
    /// Coop instanced items.
    pub coop_instanced_items: bool,
    /// Coop lives.
    pub coop_lives: bool,
    /// Coop life count.
    pub coop_num_lives: i32,
    /// Deathmatch force respawn.
    pub deathmatch_force_respawn: bool,
    /// Deathmatch no fall damage.
    pub deathmatch_no_fall_damage: bool,
    /// Deathmatch spawn farthest.
    pub deathmatch_spawn_farthest: bool,
    /// Deathmatch force respawn time.
    pub deathmatch_force_respawn_time: f64,
    /// Deathmatch allow exit.
    pub deathmatch_allow_exit: bool,
    /// Coop player collision.
    pub coop_player_collision: bool,
    /// Minimum autosave interval.
    pub auto_save_minimum_time: f64,
}

impl Default for Q2RereleaseOptions {
    fn default() -> Self {
        Self {
            coop_squad_respawn: true,
            coop_instanced_items: true,
            coop_lives: false,
            coop_num_lives: 2,
            deathmatch_force_respawn: false,
            deathmatch_no_fall_damage: false,
            deathmatch_spawn_farthest: true,
            deathmatch_force_respawn_time: 0.0,
            deathmatch_allow_exit: false,
            coop_player_collision: false,
            auto_save_minimum_time: 60.0,
        }
    }
}

/// Create rerelease options (`createQ2RereleaseOptions`).
pub fn create_q2_rerelease_options() -> Q2RereleaseOptions {
    Q2RereleaseOptions::default()
}

/// Localized print level (`Q2RereleaseEvent["localized-print"]["level"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2LocalizedPrintLevel {
    /// Low.
    Low,
    /// Medium.
    Medium,
    /// High.
    High,
    /// Chat.
    Chat,
}

/// Rerelease event (`Q2RereleaseEvent`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2RereleaseEvent {
    /// Debug shapes.
    DebugShapes {
        /// Lines.
        lines: Vec<DebugLine>,
        /// Lifetime in milliseconds.
        lifetime_milliseconds: u32,
    },
    /// World text.
    WorldText {
        /// Text.
        text: WorldTextInput,
        /// Lifetime.
        lifetime: f64,
    },
    /// Localized print.
    LocalizedPrint {
        /// Target actor.
        actor: Option<ActorId>,
        /// Level.
        level: Q2LocalizedPrintLevel,
        /// Text.
        text: String,
        /// Arguments.
        args: Vec<String>,
    },
    /// Mission objective.
    MissionObjective {
        /// Actor.
        actor: ActorId,
        /// Text.
        text: String,
        /// Arguments.
        args: Vec<String>,
        /// Whether to play the talk sound.
        talk_sound: bool,
    },
    /// Mission status.
    MissionStatus {
        /// Actor.
        actor: ActorId,
        /// Whether the icon is visible.
        icon_visible: bool,
    },
    /// Screen blend.
    ScreenBlend {
        /// Actor.
        actor: ActorId,
        /// Blend.
        blend: Vec4,
    },
    /// Help computer.
    HelpComputer {
        /// Actor.
        actor: ActorId,
        /// Whether visible.
        visible: bool,
        /// Primary objective.
        primary: String,
        /// Secondary objective.
        secondary: String,
        /// Whether to slow time.
        slow_time: bool,
    },
    /// Fog.
    Fog {
        /// Actor.
        actor: ActorId,
        /// Value.
        value: Q2FogState,
        /// Transition in milliseconds.
        transition_milliseconds: f64,
    },
    /// Flashlight.
    Flashlight {
        /// Actor.
        actor: ActorId,
        /// Whether enabled.
        enabled: bool,
        /// Hand.
        hand: Q2PlayerHand,
    },
    /// Keyed point of interest.
    KeyedPoi {
        /// Actor.
        actor: ActorId,
        /// Key.
        key: i32,
        /// Position.
        position: Vec3,
        /// Image.
        image: String,
        /// Duration.
        duration: f64,
        /// Color.
        color: i32,
        /// Flags.
        flags: i32,
    },
    /// Remove point of interest.
    RemovePoi {
        /// Actor.
        actor: ActorId,
        /// Key.
        key: i32,
    },
    /// Directional damage.
    DirectionalDamage {
        /// Actor.
        actor: ActorId,
        /// Direction.
        direction: Vec3,
        /// Damage.
        damage: f64,
        /// Whether health damage.
        health: bool,
        /// Whether armor damage.
        armor: bool,
        /// Whether shield damage.
        shield: bool,
    },
    /// Point of interest.
    Poi {
        /// Actor.
        actor: ActorId,
        /// Position.
        position: Vec3,
        /// Image.
        image: String,
        /// Duration.
        duration: f64,
        /// Color.
        color: i32,
    },
    /// Help path.
    HelpPath {
        /// Actor.
        actor: ActorId,
        /// Whether first.
        first: bool,
        /// Position.
        position: Vec3,
        /// Direction.
        direction: Vec3,
    },
    /// Coop respawn.
    CoopRespawn {
        /// Actor.
        actor: ActorId,
        /// State.
        state: Q2CoopRespawnState,
        /// Lives.
        lives: i32,
    },
    /// Autosave.
    Autosave,
    /// Alpha.
    Alpha {
        /// Actor.
        actor: ActorId,
        /// Alpha.
        alpha: f64,
    },
    /// End of unit.
    EndOfUnit {
        /// Levels.
        levels: Vec<Q2RereleaseLevelEntry>,
        /// Button time.
        button_time: f64,
    },
    /// Player dogtag.
    PlayerDogtag {
        /// Actor.
        actor: ActorId,
        /// Value.
        value: String,
    },
    /// Dynamic light.
    DynamicLight {
        /// Actor.
        actor: ActorId,
        /// Origin.
        origin: Vec3,
        /// Radius.
        radius: f64,
        /// Color.
        color: Vec3,
        /// Whether visible.
        visible: bool,
    },
    /// Restart level.
    RestartLevel {
        /// Map.
        map: String,
    },
    /// Story.
    Story {
        /// Text.
        text: String,
    },
    /// Achievement.
    Achievement {
        /// Id.
        id: String,
    },
    /// Sky.
    Sky {
        /// Name.
        name: String,
        /// Rotation.
        rotation: f64,
        /// Whether auto rotating.
        auto_rotate: bool,
        /// Axis.
        axis: Vec3,
    },
    /// Healthbar.
    Healthbar {
        /// Actor.
        actor: ActorId,
        /// Slot.
        slot: i32,
        /// Target.
        target: ActorId,
        /// Name.
        name: String,
        /// Fraction.
        fraction: f64,
        /// Whether visible.
        visible: bool,
    },
    /// Item visibility.
    ItemVisibility {
        /// Actor.
        actor: ActorId,
        /// Item.
        item: ActorId,
        /// Whether visible.
        visible: bool,
    },
}

/// Rerelease navigation result (`Q2RereleaseHooks["navigation"]`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2RereleaseNavigation {
    /// Path found.
    Path {
        /// Distance squared.
        distance_squared: f64,
        /// Points.
        points: Vec<Vec3>,
    },
    /// No navigation service.
    NoNavigation,
    /// Goal unreachable.
    Unreachable,
}

/// Rerelease player identity (`Q2RereleaseHooks["playerIdentity"]`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Q2RereleasePlayerIdentity {
    /// Seat number.
    pub seat: i32,
    /// Social id.
    pub social_id: String,
}

/// Expansion powerup timers (`Q2RereleaseHooks["expansionPowerups"]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2ExpansionPowerups {
    /// Double damage expiry.
    pub double_until: f64,
    /// Quad fire expiry.
    pub quad_fire_until: f64,
    /// IR goggles expiry.
    pub ir_until: f64,
}

/// Rerelease session hooks (`Q2RereleaseHooks`).
///
/// Seat identity, BSP hull clipping and navigation remain owned by their
/// existing engine services.
#[derive(Debug, Clone, Copy)]
pub struct Q2RereleaseHooks {
    /// Emit a rerelease event.
    pub emit: fn(&mut Q2GameServices, Q2RereleaseEvent),
    /// Read a light style.
    pub light_style: fn(&Q2GameServices, i32) -> String,
    /// Read a player identity.
    pub player_identity: fn(&Q2GameServices, ActorId) -> Q2RereleasePlayerIdentity,
    /// Clip a trigger against a player.
    pub clip_trigger: fn(ActorId, ActorId, &mut Q2GameServices) -> bool,
    /// Find a navigation path.
    pub navigation: fn(&mut Q2GameServices, Vec3, Vec3, Option<ActorId>) -> Q2RereleaseNavigation,
    /// Whether monsters are searching for a player.
    pub monsters_searching: fn(&Q2GameServices, Option<ActorId>) -> bool,
    /// Whether a monster holds a health bar.
    pub monster_holds_health_bar: Option<fn(&Q2GameServices, ActorId) -> bool>,
    /// Read expansion powerup timers.
    pub expansion_powerups: Option<fn(&mut Q2GameServices, ActorId) -> Q2ExpansionPowerups>,
    /// Clear expansion powerup timers.
    pub clear_expansion_powerups: Option<fn(ActorId, &mut Q2GameServices)>,
    /// Set player collision.
    pub player_collision: Option<fn(ActorId, &mut Q2GameServices, bool)>,
    /// Whether a player is grounded on the world.
    pub grounded_on_world: fn(ActorId, &mut Q2GameServices) -> bool,
    /// Push a player.
    pub push_player: fn(ActorId, &mut Q2GameServices, Vec3),
    /// Set actor gravity.
    pub set_actor_gravity: fn(ActorId, &mut Q2GameServices, f64),
    /// Set world gravity.
    pub set_world_gravity: fn(&mut Q2GameServices, f64),
}

/// Whether the map is a Quake 64 map (`q2IsN64`).
pub fn q2_is_n64(game: &Q2GameServices) -> bool {
    game.options.map_name.starts_with("q64/")
}

/// Whether items are instanced (`q2UsesInstancedItems`).
pub fn q2_uses_instanced_items(options: &Q2RereleaseOptions) -> bool {
    options.coop_instanced_items || options.coop_squad_respawn
}
