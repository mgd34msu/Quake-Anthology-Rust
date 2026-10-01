//! Q2 weapon shared types (`src/content/q2/foundation/weapons/types.ts`).
//!
//! Quake II `p_weapon.c` / rerelease `p_weapon.cpp` (id Software,
//! GPL-2.0-or-later). Weapon animation state is separate from actor and
//! inventory ownership.
//!
//! The donor's `Q2WeaponHooks` split along the arena boundary: `noise`
//! and `dodge` route to in-scope monster logic directly, while session
//! duties stay behind [`WeaponEngine`].

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::Vec3;

use crate::contract::ItemId;

/// Weapon owner (`Q2WeaponOwner`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2WeaponOwner {
    /// Owning actor.
    pub actor: OwnedActor,
    /// View height.
    pub view_height: f64,
}

/// Base weapon name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2BaseWeaponName {
    /// Blaster.
    Blaster,
    /// Shotgun.
    Shotgun,
    /// Super shotgun.
    Supershotgun,
    /// Machine gun.
    Machinegun,
    /// Chaingun.
    Chaingun,
    /// Hand grenades.
    Grenades,
    /// Grenade launcher.
    Grenadelauncher,
    /// Rocket launcher.
    Rocketlauncher,
    /// Hyperblaster.
    Hyperblaster,
    /// Railgun.
    Railgun,
    /// BFG.
    Bfg,
}

impl Q2BaseWeaponName {
    /// Donor spelling.
    pub fn as_str(&self) -> &'static str {
        match self {
            Q2BaseWeaponName::Blaster => "blaster",
            Q2BaseWeaponName::Shotgun => "shotgun",
            Q2BaseWeaponName::Supershotgun => "supershotgun",
            Q2BaseWeaponName::Machinegun => "machinegun",
            Q2BaseWeaponName::Chaingun => "chaingun",
            Q2BaseWeaponName::Grenades => "grenades",
            Q2BaseWeaponName::Grenadelauncher => "grenadelauncher",
            Q2BaseWeaponName::Rocketlauncher => "rocketlauncher",
            Q2BaseWeaponName::Hyperblaster => "hyperblaster",
            Q2BaseWeaponName::Railgun => "railgun",
            Q2BaseWeaponName::Bfg => "bfg",
        }
    }
}

/// Weapon name (resolves through the session source weapon registry).
pub type Q2WeaponName = String;

/// Weapon phase (`Q2WeaponPhase`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Q2WeaponPhase {
    /// Activating.
    #[default]
    Activating,
    /// Ready.
    Ready,
    /// Firing.
    Firing,
    /// Dropping.
    Dropping,
}

/// Weapon definition (`Q2WeaponDefinition`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2WeaponDefinition {
    /// Weapon name.
    pub name: Q2WeaponName,
    /// Weapon item.
    pub item: ItemId,
    /// Classname.
    pub classname: String,
    /// Ammo item.
    pub ammo: Option<ItemId>,
    /// Ammo quantity per shot.
    pub quantity: i32,
    /// Low-ammo warning threshold.
    pub warning: i32,
    /// View model.
    pub view_model: String,
    /// World model.
    pub world_model: String,
    /// Player model number.
    pub player_model: i32,
    /// Activate last frame.
    pub activate_last: i32,
    /// Fire last frame.
    pub fire_last: i32,
    /// Idle last frame.
    pub idle_last: i32,
    /// Deactivate last frame.
    pub deactivate_last: i32,
    /// Pause frames.
    pub pauses: Vec<i32>,
    /// Fire frames.
    pub fires: Vec<i32>,
    /// Whether repeating.
    pub repeating: bool,
}

/// Base weapon definition (`Q2BaseWeaponDefinition`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2BaseWeaponDefinition {
    /// Base name.
    pub name: Q2BaseWeaponName,
    /// Definition.
    pub definition: Q2WeaponDefinition,
}

/// Weapon event (`Q2WeaponEvent`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2WeaponEvent {
    /// Muzzle flash.
    Muzzleflash {
        /// Actor.
        actor: ActorId,
        /// Flash number.
        flash: i32,
        /// Whether silenced.
        silenced: bool,
    },
    /// Beam effect.
    Beam {
        /// Beam effect.
        effect: WeaponBeamEffect,
        /// Actor, when attached.
        actor: Option<ActorId>,
        /// Start.
        start: Vec3,
        /// End.
        end: Vec3,
        /// Duration.
        duration: f64,
    },
    /// View weapon.
    ViewWeapon {
        /// Actor.
        actor: ActorId,
        /// Weapon name.
        weapon: Option<Q2WeaponName>,
        /// Model path.
        model: String,
        /// Player model number.
        player_model: i32,
        /// Frame.
        frame: i32,
        /// Skin.
        skin: i32,
        /// Rate.
        rate: f64,
        /// Kick origin.
        kick_origin: Vec3,
        /// Kick angles.
        kick_angles: Vec3,
    },
    /// Player animation.
    PlayerAnimation {
        /// Actor.
        actor: ActorId,
        /// Priority.
        priority: PlayerAnimationPriority,
        /// First frame.
        first: i32,
        /// Last frame.
        last: i32,
        /// Whether to reset time.
        reset_time: bool,
    },
    /// Invisibility reveal.
    InvisibilityReveal {
        /// Actor.
        actor: ActorId,
        /// Reveal until.
        until: f64,
    },
}

/// Weapon beam effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WeaponBeamEffect {
    /// Rail trail.
    Rail,
    /// Rail water trail.
    RailWater,
    /// BFG laser.
    BfgLaser,
    /// BFG zap.
    BfgZap,
    /// Bubble trail.
    BubbleTrail,
    /// BFG lightning.
    BfgLightning,
    /// Heat beam.
    Heatbeam,
    /// Monster heat beam.
    MonsterHeatbeam,
}

/// Player animation priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayerAnimationPriority {
    /// Attack.
    Attack,
    /// Pain.
    Pain,
    /// Reverse.
    Reverse,
}

/// Lag compensation token (engine history handle).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LagToken(pub u64);

/// Session weapon duties (donor `Q2WeaponHooks` engine side).
///
/// `noise` and `dodge` are answered in-crate by monster logic, so they
/// are plain arena calls instead of engine methods.
pub trait WeaponEngine {
    /// Quad damage multiplier (donor default 4 when the hook is absent).
    fn quad_multiplier(&mut self, actor: &ActorId) -> f64;
    /// Source damage multiplier (donor default 1 when the hook is absent).
    fn source_damage_multiplier(&mut self, actor: &ActorId) -> f64;
    /// Firing interval adjustment (donor default is identity).
    fn firing_interval(&mut self, actor: &ActorId, seconds: f64) -> f64;
    /// Emit a weapon event.
    fn emit(&mut self, event: &Q2WeaponEvent);
    /// Begin lag compensation, returning a restore token when history
    /// applies (`current-world` engines return `None`).
    fn lag_begin(&mut self, actor: &ActorId, start: Vec3, direction: Vec3) -> Option<LagToken>;
    /// End lag compensation.
    fn lag_end(&mut self, token: LagToken);
    /// Report ammo changes.
    fn ammo_changed(&mut self, actor: &ActorId, ammo: &ItemId);
    /// Target eligibility (donor default is the not-self rule).
    fn can_target(&mut self, attacker: Option<&ActorId>, target: &ActorId) -> bool;
}

/// Weapon input (`Q2WeaponInput`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2WeaponInput {
    /// Attack held.
    pub attack: bool,
    /// Latched attack.
    pub latched_attack: bool,
    /// Holster.
    pub holster: bool,
    /// Angles.
    pub angles: Vec3,
    /// Ducked.
    pub ducked: bool,
    /// Spectator.
    pub spectator: bool,
    /// Notarget.
    pub notarget: bool,
    /// Hand.
    pub hand: WeaponHand,
    /// Whether to animate the player.
    pub animate_player: bool,
    /// Quad expiry.
    pub quad_until: f64,
    /// Double expiry.
    pub double_until: f64,
    /// Quad-fire expiry.
    pub quad_fire_until: f64,
    /// Haste.
    pub haste: bool,
    /// Double does not stack with quad.
    pub no_stack_double: bool,
    /// Instant switch.
    pub instant_switch: bool,
    /// Quick switch.
    pub quick_switch: bool,
    /// Infinite ammo.
    pub infinite_ammo: bool,
    /// Players collide.
    pub players_collide: bool,
    /// Gravity.
    pub gravity: f64,
    /// Weapon thunk.
    pub weapon_thunk: bool,
    /// View height.
    pub view_height: f64,
}

/// Weapon hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WeaponHand {
    /// Right hand.
    #[default]
    Right,
    /// Left hand.
    Left,
    /// Center.
    Center,
}

/// Hand grenade reservation (`Q2HandReservation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Q2HandReservation {
    /// No reservation.
    #[default]
    None,
    /// Finite reservation.
    Finite,
    /// Infinite reservation.
    Infinite,
}

/// Weapon animation state (`Q2WeaponState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2WeaponState {
    /// Primary handoff.
    pub primary_handoff: PrimaryHandoff,
    /// Current weapon.
    pub weapon: Option<Q2WeaponName>,
    /// Last weapon.
    pub last_weapon: Option<Q2WeaponName>,
    /// Pending weapon.
    pub pending: Option<Q2WeaponName>,
    /// Phase.
    pub phase: Q2WeaponPhase,
    /// Frame.
    pub frame: i32,
    /// Think time.
    pub think_time: f64,
    /// Fire finished.
    pub fire_finished: f64,
    /// Fire buffered.
    pub fire_buffered: bool,
    /// Latched attack.
    pub latched_attack: bool,
    /// Machine gun shots.
    pub machinegun_shots: i32,
    /// Empty sound time.
    pub empty_sound_time: f64,
    /// Hand reservation.
    pub hand_reservation: Q2HandReservation,
    /// Grenade time.
    pub grenade_time: f64,
    /// Grenade finished.
    pub grenade_finished: f64,
    /// Grenade blew up.
    pub grenade_blew_up: bool,
    /// Kick origin.
    pub kick_origin: Vec3,
    /// Kick angles.
    pub kick_angles: Vec3,
    /// Kick time.
    pub kick_time: f64,
    /// Kick until.
    pub kick_until: f64,
    /// Kick duration.
    pub kick_duration: f64,
    /// Loop sound.
    pub loop_sound: String,
    /// View model.
    pub view_model: Option<String>,
    /// View skin.
    pub view_skin: i32,
    /// Last firing time.
    pub last_firing_time: f64,
    /// Source firing.
    pub source_firing: bool,
    /// Gun rate.
    pub gun_rate: f64,
}

impl Q2WeaponState {
    /// Build a weapon state (`new Q2WeaponState(weapon)`).
    pub fn new(weapon: Option<Q2WeaponName>) -> Self {
        Self {
            primary_handoff: PrimaryHandoff::Active,
            weapon,
            last_weapon: None,
            pending: None,
            phase: Q2WeaponPhase::Activating,
            frame: 0,
            think_time: 0.0,
            fire_finished: 0.0,
            fire_buffered: false,
            latched_attack: false,
            machinegun_shots: 0,
            empty_sound_time: 0.0,
            hand_reservation: Q2HandReservation::None,
            grenade_time: 0.0,
            grenade_finished: 0.0,
            grenade_blew_up: false,
            kick_origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            kick_angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            kick_time: 0.0,
            kick_until: 0.0,
            kick_duration: 0.2,
            loop_sound: String::new(),
            view_model: None,
            view_skin: 0,
            last_firing_time: 0.0,
            source_firing: false,
            gun_rate: 10.0,
        }
    }
}

/// Primary handoff state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PrimaryHandoff {
    /// Active.
    #[default]
    Active,
    /// Holstering.
    Holstering,
    /// Holstered.
    Holstered,
}

/// Noise record (`Q2NoiseRecord`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2NoiseRecord {
    /// Noisy actor.
    pub actor: ActorId,
    /// Noise origin.
    pub origin: Vec3,
    /// Noise time.
    pub time: f64,
    /// Whether secondary.
    pub secondary: bool,
}

/// Grenade launch adjustment (`Q2GrenadeAdjustment`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2GrenadeAdjustment {
    /// Right offset.
    pub right: f64,
    /// Up offset.
    pub up: f64,
    /// Gravity.
    pub gravity: f64,
}

/// Quake II means of death (`MOD`).
pub struct Mod;

impl Mod {
    /// Blaster.
    pub const BLASTER: i32 = 1;
    /// Shotgun.
    pub const SHOTGUN: i32 = 2;
    /// Super shotgun.
    pub const SUPERSHOTGUN: i32 = 3;
    /// Machine gun.
    pub const MACHINEGUN: i32 = 4;
    /// Chaingun.
    pub const CHAINGUN: i32 = 5;
    /// Grenade.
    pub const GRENADE: i32 = 6;
    /// Grenade splash.
    pub const GRENADE_SPLASH: i32 = 7;
    /// Rocket.
    pub const ROCKET: i32 = 8;
    /// Rocket splash.
    pub const ROCKET_SPLASH: i32 = 9;
    /// Hyperblaster.
    pub const HYPERBLASTER: i32 = 10;
    /// Railgun.
    pub const RAILGUN: i32 = 11;
    /// BFG laser.
    pub const BFG_LASER: i32 = 12;
    /// BFG blast.
    pub const BFG_BLAST: i32 = 13;
    /// BFG effect.
    pub const BFG_EFFECT: i32 = 14;
    /// Hand grenade.
    pub const HAND_GRENADE: i32 = 15;
    /// Hand grenade splash.
    pub const HAND_GRENADE_SPLASH: i32 = 16;
    /// Held grenade.
    pub const HELD_GRENADE: i32 = 24;
    /// Melee hit.
    pub const HIT: i32 = 32;
}

/// Shot contents mask (`SHOT_MASK`).
pub const SHOT_MASK: i32 = 1 | 2 | 0x2000000 | 0x4000000;
/// Player contents (`PLAYER_CONTENTS`).
pub const PLAYER_CONTENTS: i32 = 0x40000000;
/// Projectile contents mask (`PROJECTILE_MASK`).
pub const PROJECTILE_MASK: i32 = SHOT_MASK | PLAYER_CONTENTS | 0x4000;
/// Water contents mask (`WATER_MASK`).
pub const WATER_MASK: i32 = 8 | 16 | 32;
