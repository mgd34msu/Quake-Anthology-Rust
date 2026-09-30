//! Quake III base/shared: player state.
//!
//! Donor provenance: `src/content/q3/base/shared/player-state.ts`.

use qa_core::math::{vec3, Vec3};
use qa_core::numeric::native_atof;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::shared::definitions_mirror::*;

// ---------------------------------------------------------------------------
// shared/player-state.ts (+ movement/q3/constants.ts re-exports)
// ---------------------------------------------------------------------------

/// World entity number (`ENTITYNUM_WORLD`).
pub const ENTITYNUM_WORLD: i32 = 1022;

/// No-entity number (`ENTITYNUM_NONE`).
pub const ENTITYNUM_NONE: i32 = 1023;

/// Player movement flags (`MoveFlags`, re-exported from movement constants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum MoveFlags {
    /// Ducked.
    Ducked = 1,
    /// Jump held.
    JumpHeld = 2,
    /// Backwards jump.
    BackwardsJump = 8,
    /// Backwards run.
    BackwardsRun = 16,
    /// Landing timer active.
    TimeLand = 32,
    /// Knockback timer active.
    TimeKnockback = 64,
    /// Water-jump timer active.
    TimeWaterJump = 256,
    /// Respawned.
    Respawned = 512,
    /// Use-item held.
    UseItemHeld = 1024,
    /// Grapple pull.
    GrapplePull = 2048,
    /// Following.
    Follow = 4096,
    /// Scoreboard.
    Scoreboard = 8192,
    /// Invulnerability expansion.
    InvulExpand = 16384,
}

/// User command buttons (`CommandButtons`, re-exported from movement
/// constants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum CommandButtons {
    /// Attack.
    Attack = 1,
    /// Talk.
    Talk = 2,
    /// Use holdable.
    UseHoldable = 4,
    /// Gesture.
    Gesture = 8,
    /// Walking.
    Walking = 16,
    /// Affirmative.
    Affirmative = 32,
    /// Negative.
    Negative = 64,
    /// Get flag.
    GetFlag = 128,
    /// Guard base.
    GuardBase = 256,
    /// Patrol.
    Patrol = 512,
    /// Follow me.
    FollowMe = 1024,
    /// Any.
    Any = 2048,
}

/// Player animation slots (`PlayerAnimation`, re-exported from movement
/// constants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum PlayerAnimation {
    /// First death.
    BothDeath1 = 0,
    /// First dead.
    BothDead1 = 1,
    /// Second death.
    BothDeath2 = 2,
    /// Second dead.
    BothDead2 = 3,
    /// Third death.
    BothDeath3 = 4,
    /// Third dead.
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
    /// Crouched legs walk.
    LegsWalkCr = 13,
    /// Legs walk.
    LegsWalk = 14,
    /// Legs run.
    LegsRun = 15,
    /// Legs backpedal.
    LegsBack = 16,
    /// Legs swim.
    LegsSwim = 17,
    /// Legs jump.
    LegsJump = 18,
    /// Legs land.
    LegsLand = 19,
    /// Backwards legs jump.
    LegsJumpB = 20,
    /// Backwards legs land.
    LegsLandB = 21,
    /// Legs idle.
    LegsIdle = 22,
    /// Crouched legs idle.
    LegsIdleCr = 23,
    /// Legs turn.
    LegsTurn = 24,
    /// Torso get flag.
    TorsoGetFlag = 25,
    /// Torso guard base.
    TorsoGuardBase = 26,
    /// Torso patrol.
    TorsoPatrol = 27,
    /// Torso follow me.
    TorsoFollowMe = 28,
    /// Torso affirmative.
    TorsoAffirmative = 29,
    /// Torso negative.
    TorsoNegative = 30,
    /// Crouched backpedal.
    LegsBackCr = 32,
    /// Backwards legs walk.
    LegsBackWalk = 33,
    /// Flag run.
    FlagRun = 34,
    /// Flag stand.
    FlagStand = 35,
    /// Flag stand-to-run.
    FlagStand2Run = 36,
}

/// User command (`UserCommand`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UserCommand {
    /// Server time.
    pub server_time: i32,
    /// Command angles.
    pub angles: Vec3,
    /// Buttons.
    pub buttons: i32,
    /// Selected weapon tag.
    pub weapon: i32,
    /// Forward movement.
    pub forwardmove: i32,
    /// Right movement.
    pub rightmove: i32,
    /// Up movement.
    pub upmove: i32,
}

/// Fixed source slot storage owned elsewhere (`PlayerSlotBinding`).
pub trait PlayerSlotBinding {
    /// Read a slot.
    fn read(&self, index: usize) -> i32;
    /// Write a slot.
    fn write(&self, index: usize, value: i32);
}

/// Fixed source arrays with checked access, including unused protocol slots
/// (`PlayerStateSlots`).
#[derive(Clone)]
pub struct PlayerStateSlots {
    values: Vec<i32>,
    binding: Option<Rc<dyn PlayerSlotBinding>>,
    stored: Option<Rc<dyn Fn(usize, i32)>>,
}

impl std::fmt::Debug for PlayerStateSlots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlayerStateSlots")
            .field("values", &self.values)
            .field("bound", &self.binding.is_some())
            .finish()
    }
}

impl PlayerStateSlots {
    /// Fixed slots, optionally over source values, an owner binding, and a
    /// store notification.
    ///
    /// # Panics
    ///
    /// Panics when `source_values` does not hold exactly `length` values.
    #[must_use]
    pub fn new(
        length: usize,
        source_values: Option<Vec<i32>>,
        binding: Option<Rc<dyn PlayerSlotBinding>>,
        stored: Option<Rc<dyn Fn(usize, i32)>>,
    ) -> Self {
        if let Some(values) = source_values {
            assert!(
                values.len() == length,
                "Player state source slots require {length} values, got {}",
                values.len()
            );
            Self {
                values,
                binding,
                stored,
            }
        } else {
            Self {
                values: vec![0; length],
                binding,
                stored,
            }
        }
    }

    /// Slot count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether there are no slots.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Read a slot.
    ///
    /// # Panics
    ///
    /// Panics when `index` is outside the slots.
    #[must_use]
    pub fn get(&self, index: usize) -> i32 {
        if let Some(binding) = &self.binding {
            if index < self.values.len() {
                return binding.read(index);
            }
            panic!("Player state slot {index} outside {}", self.values.len());
        }
        *self
            .values
            .get(index)
            .unwrap_or_else(|| panic!("Player state slot {index} outside {}", self.values.len()))
    }

    /// Write a slot, then report the stored value.
    ///
    /// # Panics
    ///
    /// Panics when `index` is outside the slots.
    pub fn set(&mut self, index: usize, value: i32) {
        if index >= self.values.len() {
            panic!("Player state slot {index} outside {}", self.values.len());
        }
        if let Some(binding) = &self.binding {
            binding.write(index, value);
        } else {
            self.values[index] = value;
        }
        if let Some(stored) = &self.stored {
            let current = self.get(index);
            stored(index, current);
        }
    }

    /// Copy every slot value.
    #[must_use]
    pub fn copy(&self) -> Vec<i32> {
        (0..self.values.len()).map(|index| self.get(index)).collect()
    }
}

pub(crate) fn copy_slots(target: &mut PlayerStateSlots, source: &PlayerStateSlots) {
    for index in 0..target.len() {
        target.set(index, source.get(index));
    }
}

/// Predictable event record (`PredictableEvent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PredictableEvent {
    /// Event sequence at capture.
    pub sequence: i32,
    /// Event tag.
    pub event: i32,
    /// Event parameter.
    pub parameter: i32,
}

/// Event-debug module (`PredictableEventDebug` module word).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventDebugModule {
    /// Game module.
    Game,
    /// Client-game module.
    Cgame,
}

/// Source event-debug sink (`PredictableEventDebug`).
pub trait PredictableEventDebug {
    /// Owning module.
    fn module(&self) -> EventDebugModule;
    /// `showevents` cvar text.
    fn show_events(&self) -> String;
    /// Print a debug line.
    fn print(&self, message: &str);
}

// `bg_misc.c:eventnames`, including its omitted `EV_OBELISKPAIN` entry.
pub(crate) const EVENT_NAMES: [&str; 76] = [
    "EV_NONE",
    "EV_FOOTSTEP",
    "EV_FOOTSTEP_METAL",
    "EV_FOOTSPLASH",
    "EV_FOOTWADE",
    "EV_SWIM",
    "EV_STEP_4",
    "EV_STEP_8",
    "EV_STEP_12",
    "EV_STEP_16",
    "EV_FALL_SHORT",
    "EV_FALL_MEDIUM",
    "EV_FALL_FAR",
    "EV_JUMP_PAD",
    "EV_JUMP",
    "EV_WATER_TOUCH",
    "EV_WATER_LEAVE",
    "EV_WATER_UNDER",
    "EV_WATER_CLEAR",
    "EV_ITEM_PICKUP",
    "EV_GLOBAL_ITEM_PICKUP",
    "EV_NOAMMO",
    "EV_CHANGE_WEAPON",
    "EV_FIRE_WEAPON",
    "EV_USE_ITEM0",
    "EV_USE_ITEM1",
    "EV_USE_ITEM2",
    "EV_USE_ITEM3",
    "EV_USE_ITEM4",
    "EV_USE_ITEM5",
    "EV_USE_ITEM6",
    "EV_USE_ITEM7",
    "EV_USE_ITEM8",
    "EV_USE_ITEM9",
    "EV_USE_ITEM10",
    "EV_USE_ITEM11",
    "EV_USE_ITEM12",
    "EV_USE_ITEM13",
    "EV_USE_ITEM14",
    "EV_USE_ITEM15",
    "EV_ITEM_RESPAWN",
    "EV_ITEM_POP",
    "EV_PLAYER_TELEPORT_IN",
    "EV_PLAYER_TELEPORT_OUT",
    "EV_GRENADE_BOUNCE",
    "EV_GENERAL_SOUND",
    "EV_GLOBAL_SOUND",
    "EV_GLOBAL_TEAM_SOUND",
    "EV_BULLET_HIT_FLESH",
    "EV_BULLET_HIT_WALL",
    "EV_MISSILE_HIT",
    "EV_MISSILE_MISS",
    "EV_MISSILE_MISS_METAL",
    "EV_RAILTRAIL",
    "EV_SHOTGUN",
    "EV_BULLET",
    "EV_PAIN",
    "EV_DEATH1",
    "EV_DEATH2",
    "EV_DEATH3",
    "EV_OBITUARY",
    "EV_POWERUP_QUAD",
    "EV_POWERUP_BATTLESUIT",
    "EV_POWERUP_REGEN",
    "EV_GIB_PLAYER",
    "EV_SCOREPLUM",
    "EV_PROXIMITY_MINE_STICK",
    "EV_PROXIMITY_MINE_TRIGGER",
    "EV_KAMIKAZE",
    "EV_OBELISKEXPLODE",
    "EV_INVUL_IMPACT",
    "EV_JUICED",
    "EV_LIGHTNINGBOLT",
    "EV_DEBUG_LINE",
    "EV_STOPLOOPINGSOUND",
    "EV_TAUNT",
];

/// Owned `playerState_t` storage binding (`PlayerAuthorityBinding`).
///
/// The engine transports game-defined integer words unchanged.
pub trait PlayerAuthorityBinding {
    /// Read the authoritative origin.
    fn origin(&self) -> Vec3;
    /// Write the authoritative origin.
    fn set_origin(&self, value: Vec3);
    /// Read the authoritative velocity.
    fn read_velocity(&self) -> Vec3;
    /// Write the authoritative velocity.
    fn set_velocity(&self, value: Vec3);
    /// Stat slot binding.
    fn stats(&self) -> Rc<dyn PlayerSlotBinding>;
    /// Ammunition slot binding.
    fn ammo(&self) -> Rc<dyn PlayerSlotBinding>;
}

/// Authority handling for [`PlayerState::copy_from`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityStores {
    /// Keep authority-owned words (origin, velocity, health/armor/weapon
    /// stats, ammunition).
    PreserveAuthority,
    /// Copy authority-owned words as well.
    ReplaceAuthority,
}

/// Owned `playerState_t` storage (`PlayerStateRecord`).
///
/// Movement, weapon, and weapon-state tags are raw `i32` words, matching
/// the donor's `SourcePlayerState` layer used by game logic; the typed
/// `PlayerState<MoveType, Weapon, WeaponState>` generic is TypeScript-only
/// machinery over the same storage.
#[derive(Clone)]
pub struct PlayerState {
    product: Product,
    authority: Option<Rc<dyn PlayerAuthorityBinding>>,
    event_debug: Option<Rc<dyn PredictableEventDebug>>,
    origin_value: Vec3,
    velocity_value: Vec3,
    /// Command time.
    pub command_time: i32,
    /// Movement type tag.
    pub pm_type: i32,
    /// Bob cycle.
    pub bob_cycle: i32,
    /// Movement flags.
    pub pm_flags: i32,
    /// Movement timer.
    pub pm_time: i32,
    /// Weapon time.
    pub weapon_time: i32,
    /// Gravity.
    pub gravity: i32,
    /// Speed.
    pub speed: i32,
    /// Delta angles.
    pub delta_angles: Vec3,
    /// Ground entity number.
    pub ground_entity_num: i32,
    /// Legs timer.
    pub legs_timer: i32,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso timer.
    pub torso_timer: i32,
    /// Torso animation.
    pub torso_anim: i32,
    /// Movement direction.
    pub movement_dir: i32,
    /// Grapple point.
    pub grapple_point: Vec3,
    /// Entity flags.
    pub e_flags: i32,
    /// Event sequence.
    pub event_sequence: i32,
    /// Predictable events.
    pub events: PlayerStateSlots,
    /// Predictable event parameters.
    pub event_parms: PlayerStateSlots,
    /// External event.
    pub external_event: i32,
    /// External event parameter.
    pub external_event_parm: i32,
    /// External event time.
    pub external_event_time: i32,
    /// Client number.
    pub client_num: i32,
    /// Weapon tag.
    pub weapon: i32,
    /// Weapon state tag.
    pub weapon_state: i32,
    /// View angles.
    pub viewangles: Vec3,
    /// View height.
    pub viewheight: i32,
    /// Damage event.
    pub damage_event: i32,
    /// Damage yaw.
    pub damage_yaw: i32,
    /// Damage pitch.
    pub damage_pitch: i32,
    /// Damage count.
    pub damage_count: i32,
    /// Stats.
    pub stats: PlayerStateSlots,
    /// Persistant stats.
    pub persistant: PlayerStateSlots,
    /// Powerups.
    pub powerups: PlayerStateSlots,
    /// Ammunition.
    pub ammo: PlayerStateSlots,
    /// Generic value.
    pub generic1: i32,
    /// Loop sound.
    pub loop_sound: i32,
    /// Jump-pad entity.
    pub jumppad_ent: i32,
    /// Ping.
    pub ping: i32,
    /// Player-move frame count.
    pub pmove_framecount: i32,
    /// Jump-pad frame.
    pub jumppad_frame: i32,
    /// Consumed entity event sequence.
    pub entity_event_sequence: i32,
}

impl std::fmt::Debug for PlayerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlayerState")
            .field("product", &self.product)
            .field("command_time", &self.command_time)
            .field("pm_type", &self.pm_type)
            .field("event_sequence", &self.event_sequence)
            .field("client_num", &self.client_num)
            .field("weapon", &self.weapon)
            .finish()
    }
}

/// Source player state layer (`SourcePlayerState`).
pub type SourcePlayerState = PlayerState;

impl PlayerState {
    /// Zero player state with normal movement, no weapon, and a ready
    /// weapon (`PlayerState` construction).
    #[must_use]
    pub fn new(product: Product, authority: Option<Rc<dyn PlayerAuthorityBinding>>) -> Self {
        let stats = PlayerStateSlots::new(16, None, authority.as_ref().map(|binding| binding.stats()), None);
        let ammo = PlayerStateSlots::new(16, None, authority.as_ref().map(|binding| binding.ammo()), None);
        let zero = vec3(0.0, 0.0, 0.0);
        Self {
            product,
            authority,
            event_debug: None,
            origin_value: zero,
            velocity_value: zero,
            command_time: 0,
            pm_type: MoveType::PmNormal as i32,
            bob_cycle: 0,
            pm_flags: 0,
            pm_time: 0,
            weapon_time: 0,
            gravity: 0,
            speed: 0,
            delta_angles: zero,
            ground_entity_num: 0,
            legs_timer: 0,
            legs_anim: 0,
            torso_timer: 0,
            torso_anim: 0,
            movement_dir: 0,
            grapple_point: zero,
            e_flags: 0,
            event_sequence: 0,
            events: PlayerStateSlots::new(2, None, None, None),
            event_parms: PlayerStateSlots::new(2, None, None, None),
            external_event: 0,
            external_event_parm: 0,
            external_event_time: 0,
            client_num: 0,
            weapon: Weapon::WpNone as i32,
            weapon_state: WeaponState::WeaponReady as i32,
            viewangles: zero,
            viewheight: 0,
            damage_event: 0,
            damage_yaw: 0,
            damage_pitch: 0,
            damage_count: 0,
            stats,
            persistant: PlayerStateSlots::new(16, None, None, None),
            powerups: PlayerStateSlots::new(16, None, None, None),
            ammo,
            generic1: 0,
            loop_sound: 0,
            jumppad_ent: 0,
            ping: 0,
            pmove_framecount: 0,
            jumppad_frame: 0,
            entity_event_sequence: 0,
        }
    }

    /// Owning product.
    #[must_use]
    pub fn product(&self) -> Product {
        self.product
    }

    /// Origin, from the authority when bound.
    #[must_use]
    pub fn origin(&self) -> Vec3 {
        self.authority
            .as_ref()
            .map_or(self.origin_value, |binding| binding.origin())
    }

    /// Write the origin, through the authority when bound.
    pub fn set_origin(&mut self, value: Vec3) {
        if let Some(binding) = &self.authority {
            binding.set_origin(value);
        } else {
            self.origin_value = value;
        }
    }

    /// Velocity, from the authority when bound.
    #[must_use]
    pub fn velocity(&self) -> Vec3 {
        self.authority
            .as_ref()
            .map_or(self.velocity_value, |binding| binding.read_velocity())
    }

    /// Write the velocity, through the authority when bound.
    pub fn set_velocity(&mut self, value: Vec3) {
        if let Some(binding) = &self.authority {
            binding.set_velocity(value);
        } else {
            self.velocity_value = value;
        }
    }

    /// Health from the product's health stat slot.
    #[must_use]
    pub fn health(&self) -> i32 {
        self.stats.get(stat_schema(self.product).indices().health)
    }

    /// Write health to the product's health stat slot.
    pub fn set_health(&mut self, value: i32) {
        let slot = stat_schema(self.product).indices().health;
        self.stats.set(slot, value);
    }

    /// Install or clear event debugging.
    pub fn set_event_debug(&mut self, debug: Option<Rc<dyn PredictableEventDebug>>) {
        self.event_debug = debug;
    }

    /// Deep copy without authority.
    #[must_use]
    pub fn copy(&self) -> Self {
        let mut result = Self::new(self.product, None);
        result.copy_from(self, AuthorityStores::ReplaceAuthority);
        result
    }

    /// Copy every field from a source state.
    pub fn copy_from(&mut self, source: &PlayerState, stores: AuthorityStores) {
        let preserve = self.authority.is_some() && stores == AuthorityStores::PreserveAuthority;
        self.product = source.product;
        self.command_time = source.command_time;
        self.pm_type = source.pm_type;
        self.bob_cycle = source.bob_cycle;
        self.pm_flags = source.pm_flags;
        self.pm_time = source.pm_time;
        if !preserve {
            self.set_origin(source.origin());
            self.set_velocity(source.velocity());
        }
        self.weapon_time = source.weapon_time;
        self.gravity = source.gravity;
        self.speed = source.speed;
        self.delta_angles = source.delta_angles;
        self.ground_entity_num = source.ground_entity_num;
        self.legs_timer = source.legs_timer;
        self.legs_anim = source.legs_anim;
        self.torso_timer = source.torso_timer;
        self.torso_anim = source.torso_anim;
        self.movement_dir = source.movement_dir;
        self.grapple_point = source.grapple_point;
        self.e_flags = source.e_flags;
        self.event_sequence = source.event_sequence;
        copy_slots(&mut self.events, &source.events);
        copy_slots(&mut self.event_parms, &source.event_parms);
        self.external_event = source.external_event;
        self.external_event_parm = source.external_event_parm;
        self.external_event_time = source.external_event_time;
        self.client_num = source.client_num;
        self.weapon = source.weapon;
        self.weapon_state = source.weapon_state;
        self.viewangles = source.viewangles;
        self.viewheight = source.viewheight;
        self.damage_event = source.damage_event;
        self.damage_yaw = source.damage_yaw;
        self.damage_pitch = source.damage_pitch;
        self.damage_count = source.damage_count;
        let schema = stat_schema(self.product);
        let indices = *schema.indices();
        for index in 0..self.stats.len() {
            if preserve && (index == indices.health || index == indices.armor || index == indices.weapons) {
                continue;
            }
            self.stats.set(index, source.stats.get(index));
        }
        copy_slots(&mut self.persistant, &source.persistant);
        copy_slots(&mut self.powerups, &source.powerups);
        if !preserve {
            copy_slots(&mut self.ammo, &source.ammo);
        }
        self.generic1 = source.generic1;
        self.loop_sound = source.loop_sound;
        self.jumppad_ent = source.jumppad_ent;
        self.ping = source.ping;
        self.pmove_framecount = source.pmove_framecount;
        self.jumppad_frame = source.jumppad_frame;
        self.entity_event_sequence = source.entity_event_sequence;
    }

    /// Queue a predictable event (`BG_AddPredictableEventToPlayerstate`).
    ///
    /// # Panics
    ///
    /// Panics when event debugging is enabled and `event` has no
    /// `bg_misc.c` event name.
    pub fn add_event(&mut self, event: i32, parameter: i32) -> PredictableEvent {
        if let Some(debug) = &self.event_debug {
            let text: String = debug.show_events().chars().take(255).collect();
            if native_atof(&text).is_ok_and(|value| value != 0.0) {
                let name = usize::try_from(event)
                    .ok()
                    .and_then(|index| EVENT_NAMES.get(index))
                    .unwrap_or_else(|| panic!("bg_misc.c eventnames has no entry for {event}"));
                let label = match debug.module() {
                    EventDebugModule::Game => " game",
                    EventDebugModule::Cgame => "Cgame",
                };
                debug.print(&format!(
                    "{label} event svt {:>5} -> {:>5}: num = {name:>20} parm {parameter}\n",
                    self.pmove_framecount, self.event_sequence
                ));
            }
        }
        let sequence = self.event_sequence;
        let slot = (sequence & 1) as usize;
        self.events.set(slot, event);
        self.event_parms.set(slot, parameter);
        self.event_sequence = sequence.wrapping_add(1);
        PredictableEvent {
            sequence,
            event,
            parameter,
        }
    }
}

/// Create a retail zero player state (`createPlayerState`).
#[must_use]
pub fn create_player_state(product: Product, authority: Option<Rc<dyn PlayerAuthorityBinding>>) -> PlayerState {
    PlayerState::new(product, authority)
}
