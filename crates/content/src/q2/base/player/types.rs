//! Q2 player types (`src/content/q2/base/player/types.ts`).
//!
//! Player behavior adapted from id Software Quake II p_client.c, p_view.c,
//! p_hud.c and g_cmds.c.

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{Bounds, Vec3, Vec4};

use crate::contract::{ArmorState, InventoryEntry, ItemId};
use crate::q2::foundation::host::{Q2Edition, Q2GameServices, Q2Mode, Q2PresentationEvent};
use crate::q2::foundation::items::Q2PlayerPowerups;
use crate::q2::foundation::weapons::types::{Q2WeaponInput, Q2WeaponName};
use crate::q2::support::contracts::{AttackProvenance, BodyState, CombatState};

/// Player movement observation (`Q2PlayerMovement`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PlayerMovement {
    /// View angles.
    pub view_angles: Vec3,
    /// Command angles.
    pub command_angles: Vec3,
    /// Water level.
    pub water_level: i32,
    /// Water contents.
    pub water_type: i32,
    /// Whether grounded.
    pub grounded: bool,
    /// Whether ducked.
    pub ducked: bool,
    /// Buttons.
    pub buttons: i32,
    /// Standing bounds.
    pub standing_bounds: Bounds,
    /// Whether to animate as Q2.
    pub animate_q2: bool,
}

/// Player movement change (`Q2PlayerMovementChange`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2PlayerMovementChange {
    /// Spawn.
    Spawn(Q2PlayerSpawnChange),
    /// Teleport.
    Teleport(Q2PlayerSpawnChange),
    /// Freeze.
    Freeze {
        /// Origin.
        origin: Vec3,
        /// Angles.
        angles: Vec3,
    },
    /// Noclip.
    Noclip {
        /// Whether enabled.
        enabled: bool,
    },
}

/// Spawn/teleport movement change.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PlayerSpawnChange {
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Command angles.
    pub command_angles: Vec3,
    /// Hold milliseconds.
    pub hold_milliseconds: i32,
    /// Whether spectator.
    pub spectator: bool,
}

/// Player view (`Q2PlayerView`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PlayerView {
    /// Angles.
    pub angles: Vec3,
    /// Offset.
    pub offset: Vec3,
    /// Kick angles.
    pub kick_angles: Vec3,
    /// Gun angles.
    pub gun_angles: Vec3,
    /// Gun offset.
    pub gun_offset: Vec3,
    /// Blend.
    pub blend: Vec4,
    /// Field of view.
    pub fov: i32,
    /// Whether underwater.
    pub underwater: bool,
    /// Flashes.
    pub flashes: i32,
    /// Health.
    pub health: f64,
    /// Armor.
    pub armor: f64,
    /// Ammo.
    pub ammo: f64,
    /// Score.
    pub score: i32,
    /// Selected item.
    pub selected_item: Option<ItemId>,
    /// Powerup timer.
    pub timer: Option<Q2PlayerTimer>,
    /// Whether spectator.
    pub spectator: bool,
    /// Layout bits.
    pub layouts: i32,
}

/// Player powerup timer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Q2PlayerTimer {
    /// Item.
    pub item: ItemId,
    /// Seconds.
    pub seconds: i32,
}

/// Scoreboard row (`Q2ScoreRow`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Q2ScoreRow {
    /// Slot.
    pub slot: i32,
    /// Name.
    pub name: String,
    /// Score.
    pub score: i32,
    /// Ping.
    pub ping: i32,
    /// Minutes.
    pub minutes: i32,
    /// Whether spectator.
    pub spectator: bool,
}

/// Player print level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2PrintLevel {
    /// Low.
    Low,
    /// Medium.
    Medium,
    /// High.
    High,
    /// Chat.
    Chat,
}

/// Player event (`Q2PlayerEvent`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2PlayerEvent {
    /// Print.
    Print {
        /// Target.
        target: Option<ActorId>,
        /// Level.
        level: Q2PrintLevel,
        /// Text.
        text: String,
    },
    /// Userinfo.
    Userinfo {
        /// Actor.
        actor: ActorId,
        /// Slot.
        slot: i32,
        /// Name.
        name: String,
        /// Skin.
        skin: String,
    },
    /// Stuff text.
    StuffText {
        /// Actor.
        actor: ActorId,
        /// Text.
        text: String,
    },
    /// View.
    View {
        /// Actor.
        actor: ActorId,
        /// View.
        view: Q2PlayerView,
    },
    /// Scoreboard.
    Scoreboard {
        /// Actor.
        actor: ActorId,
        /// Rows.
        rows: Vec<Q2ScoreRow>,
        /// Killer.
        killer: Option<ActorId>,
        /// Whether reliable.
        reliable: bool,
    },
    /// Inventory.
    Inventory {
        /// Actor.
        actor: ActorId,
        /// Entries.
        entries: Vec<InventoryEntry>,
        /// Whether visible.
        visible: Option<bool>,
        /// Selected item.
        selected: Option<Option<ItemId>>,
        /// Labels.
        labels: Option<Vec<Q2ItemLabel>>,
    },
    /// Help.
    Help {
        /// Actor.
        actor: ActorId,
        /// Whether visible.
        visible: bool,
    },
    /// Load menu.
    LoadMenu {
        /// Actor.
        actor: ActorId,
    },
    /// Trail.
    Trail {
        /// Actor.
        actor: ActorId,
        /// Origin.
        origin: Vec3,
        /// Time.
        time: f64,
    },
    /// Chase.
    Chase {
        /// Actor.
        actor: ActorId,
        /// Target.
        target: Option<ActorId>,
    },
}

/// Inventory item label.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Q2ItemLabel {
    /// Item.
    pub item: ItemId,
    /// Name.
    pub name: String,
}

/// Arsenal category (`grantSelectedArsenal` category).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2ArsenalCategory {
    /// Weapons.
    Weapons,
    /// Ammo.
    Ammo,
}

/// Spawn placement (`selectSpawn` result).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2SpawnPlacement {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
}

/// Score-a-death hook function.
pub type Q2PlayerScoreHook = fn(ActorId, Option<ActorId>, &mut Q2GameServices, i32, i32, ActorId);

/// Client-command hook function.
pub type Q2PlayerCommandHook = fn(ActorId, &mut Q2GameServices, &str, &[String]) -> bool;

/// Player hooks (`Q2PlayerHooks`).
#[derive(Debug, Clone, Copy)]
pub struct Q2PlayerHooks {
    /// Quad-fire drop expiry.
    pub quad_fire_drop_until: Option<fn(ActorId) -> f64>,
    /// Grant the selected arsenal.
    pub grant_selected_arsenal: Option<fn(ActorId, Q2ArsenalCategory) -> bool>,
    /// Give the selected item.
    pub give_selected_item: Option<fn(ActorId, &[String]) -> bool>,
    /// Read weapon state.
    pub weapon_state: Option<fn(ActorId) -> Option<Q2CharacterWeapon>>,
    /// Read movement.
    pub movement: fn(ActorId) -> Q2PlayerMovement,
    /// Apply a movement change.
    pub set_movement: fn(ActorId, Q2PlayerMovementChange),
    /// Emit a player event.
    pub emit: fn(Q2PlayerEvent),
    /// Emit noise.
    pub noise: fn(ActorId, Vec3),
    /// Read weapon input.
    pub weapon_input: fn(ActorId) -> Q2WeaponInput,
    /// Whether an address is banned.
    pub banned: fn(&str) -> bool,
    /// Score a death.
    pub score: Option<Q2PlayerScoreHook>,
    /// React to a spawn.
    pub player_spawned: Option<fn(ActorId, &mut Q2GameServices)>,
    /// React to persistent inventory setup.
    pub persistent_inventory_initialized: Option<fn(ActorId, &mut Q2GameServices)>,
    /// Select a spawn.
    pub select_spawn: Option<fn(ActorId, &mut Q2GameServices) -> Option<Q2SpawnPlacement>>,
    /// React to a death.
    pub death: Option<fn(ActorId, &mut Q2GameServices, Option<AttackProvenance>)>,
    /// Drop foreign inventory.
    pub drop_inventory: Option<fn(ActorId, &mut Q2GameServices, Option<AttackProvenance>)>,
    /// React before death inventory clears.
    pub before_death_inventory: Option<fn(ActorId, &mut Q2GameServices, Option<AttackProvenance>)>,
    /// React to a disconnect.
    pub disconnect: Option<fn(ActorId, &mut Q2GameServices)>,
    /// Handle a client command.
    pub command: Option<Q2PlayerCommandHook>,
}

/// Player rules (`Q2PlayerRules`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PlayerRules {
    /// Password.
    pub password: String,
    /// Spectator password.
    pub spectator_password: String,
    /// Maximum spectators.
    pub max_spectators: i32,
    /// Whether cheats are enabled.
    pub cheats: bool,
    /// Time limit in minutes.
    pub time_limit_minutes: i32,
    /// Frag limit.
    pub frag_limit: i32,
    /// Map list.
    pub map_list: Vec<String>,
    /// Whether to shuffle the map list.
    pub map_list_shuffle: bool,
    /// Next map.
    pub next_map: String,
    /// Spawn point.
    pub spawn_point: String,
    /// Flood messages.
    pub flood_messages: i32,
    /// Flood window seconds.
    pub flood_seconds: f64,
    /// Flood wait seconds.
    pub flood_wait_seconds: f64,
    /// Roll speed.
    pub roll_speed: f64,
    /// Roll angle.
    pub roll_angle: f64,
    /// Run pitch.
    pub run_pitch: f64,
    /// Run roll.
    pub run_roll: f64,
    /// Bob up.
    pub bob_up: f64,
    /// Bob pitch.
    pub bob_pitch: f64,
    /// Bob roll.
    pub bob_roll: f64,
    /// Gun offset.
    pub gun_offset: Vec3,
}

impl Default for Q2PlayerRules {
    fn default() -> Self {
        Q2PlayerRules {
            password: String::new(),
            spectator_password: String::new(),
            max_spectators: 4,
            cheats: false,
            time_limit_minutes: 0,
            frag_limit: 0,
            map_list: Vec::new(),
            map_list_shuffle: false,
            next_map: String::new(),
            spawn_point: String::new(),
            flood_messages: 4,
            flood_seconds: 4.0,
            flood_wait_seconds: 10.0,
            roll_speed: 200.0,
            roll_angle: 2.0,
            run_pitch: 0.002,
            run_roll: 0.005,
            bob_up: 0.005,
            bob_pitch: 0.002,
            bob_roll: 0.002,
            gun_offset: Vec3::default(),
        }
    }
}

/// Create player rules (`createQ2PlayerRules`).
pub fn create_q2_player_rules() -> Q2PlayerRules {
    Q2PlayerRules::default()
}

/// Player carry (`Q2PlayerCarry`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PlayerCarry {
    /// Health.
    pub health: f64,
    /// Maximum health.
    pub maximum_health: f64,
    /// Armor.
    pub armor: ArmorState,
    /// Inventory.
    pub inventory: Vec<InventoryEntry>,
    /// Weapon.
    pub weapon: Option<Q2WeaponName>,
    /// Selected item.
    pub selected_item: Option<ItemId>,
    /// Score.
    pub score: i32,
    /// Flags.
    pub flags: i64,
    /// Power cubes.
    pub power_cubes: i32,
}

/// Player gender.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2PlayerGender {
    /// Male.
    Male,
    /// Female.
    Female,
    /// Neutral.
    Neutral,
}

/// Player hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2PlayerHand {
    /// Right.
    Right,
    /// Left.
    Left,
    /// Center.
    Center,
}

/// Player state (`Q2PlayerState`).
///
/// Fields here belong to Q2's player rules/presentation, never duplicate
/// shared live health or inventory.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PlayerState {
    /// Slot.
    pub slot: i32,
    /// Entered-at time.
    pub entered_at: f64,
    /// Whether Q2 weapons drive this player.
    pub use_q2_weapons: bool,
    /// Whether Q2 inventory drives this player.
    pub use_q2_inventory: bool,
    /// Spawn inventory.
    pub spawn_inventory: Vec<InventoryEntry>,
    /// Userinfo.
    pub userinfo: String,
    /// Name.
    pub name: String,
    /// Skin.
    pub skin: String,
    /// Gender.
    pub gender: Q2PlayerGender,
    /// Field of view.
    pub fov: i32,
    /// Hand.
    pub hand: Q2PlayerHand,
    /// Whether spectator.
    pub spectator: bool,
    /// Whether spectator was requested.
    pub requested_spectator: bool,
    /// Whether connected.
    pub connected: bool,
    /// Whether dead.
    pub dead: bool,
    /// Whether gibbed.
    pub gibbed: bool,
    /// Whether noclip.
    pub noclip: bool,
    /// Whether god mode.
    pub god: bool,
    /// Whether notarget.
    pub notarget: bool,
    /// Score.
    pub score: i32,
    /// Ping.
    pub ping: i32,
    /// Respawn time.
    pub respawn_time: f64,
    /// Air finished.
    pub air_finished: f64,
    /// Next drown time.
    pub next_drown_time: f64,
    /// Drown damage.
    pub drown_damage: f64,
    /// Old water level.
    pub old_water_level: i32,
    /// Breather sound toggle.
    pub breather_sound: i32,
    /// Pain debounce.
    pub pain_debounce: f64,
    /// Damage blood.
    pub damage_blood: f64,
    /// Damage armor.
    pub damage_armor: f64,
    /// Damage power armor.
    pub damage_power_armor: f64,
    /// Damage knockback.
    pub damage_knockback: f64,
    /// Damage from.
    pub damage_from: Vec3,
    /// Damage blend.
    pub damage_blend: Vec3,
    /// Damage alpha.
    pub damage_alpha: f64,
    /// Bonus alpha.
    pub bonus_alpha: f64,
    /// Damage pitch.
    pub damage_pitch: f64,
    /// Damage roll.
    pub damage_roll: f64,
    /// Damage time.
    pub damage_time: f64,
    /// Power armor time.
    pub power_armor_time: f64,
    /// Fall time.
    pub fall_time: f64,
    /// Fall value.
    pub fall_value: f64,
    /// Whether in landmark free fall.
    pub landmark_free_fall: bool,
    /// Landmark noise time.
    pub landmark_noise_time: f64,
    /// Old velocity.
    pub old_velocity: Vec3,
    /// Old view angles.
    pub old_view_angles: Vec3,
    /// Killer yaw.
    pub killer_yaw: f64,
    /// Buttons.
    pub buttons: i32,
    /// Latched buttons.
    pub latched_buttons: i32,
    /// Weapon thunk.
    pub weapon_thunk: bool,
    /// Bob time.
    pub bob_time: f64,
    /// Bob move.
    pub bob_move: f64,
    /// Event.
    pub event: String,
    /// Animation priority.
    pub animation_priority: i32,
    /// Animation end.
    pub animation_end: i32,
    /// Animation duck.
    pub animation_duck: bool,
    /// Animation run.
    pub animation_run: bool,
    /// Loop sound.
    pub loop_sound: String,
    /// Selected item.
    pub selected_item: Option<ItemId>,
    /// Whether scores are shown.
    pub show_scores: bool,
    /// Whether inventory is shown.
    pub show_inventory: bool,
    /// Whether help is shown.
    pub show_help: bool,
    /// Chase target.
    pub chase_target: Option<ActorId>,
    /// Coop respawn carry.
    pub coop_respawn: Option<Q2PlayerCarry>,
    /// Flood times.
    pub flood_times: Vec<f64>,
    /// Flood lock expiry.
    pub flood_lock_until: f64,
}

impl Q2PlayerState {
    /// Build a player state.
    pub fn new(slot: i32, entered_at: f64) -> Self {
        Q2PlayerState {
            slot,
            entered_at,
            use_q2_weapons: true,
            use_q2_inventory: true,
            spawn_inventory: Vec::new(),
            userinfo: String::new(),
            name: String::new(),
            skin: "male/grunt".to_string(),
            gender: Q2PlayerGender::Male,
            fov: 90,
            hand: Q2PlayerHand::Right,
            spectator: false,
            requested_spectator: false,
            connected: true,
            dead: false,
            gibbed: false,
            noclip: false,
            god: false,
            notarget: false,
            score: 0,
            ping: 0,
            respawn_time: 0.0,
            air_finished: 0.0,
            next_drown_time: 0.0,
            drown_damage: 2.0,
            old_water_level: 0,
            breather_sound: 0,
            pain_debounce: 0.0,
            damage_blood: 0.0,
            damage_armor: 0.0,
            damage_power_armor: 0.0,
            damage_knockback: 0.0,
            damage_from: Vec3::default(),
            damage_blend: Vec3::default(),
            damage_alpha: 0.0,
            bonus_alpha: 0.0,
            damage_pitch: 0.0,
            damage_roll: 0.0,
            damage_time: 0.0,
            power_armor_time: 0.0,
            fall_time: 0.0,
            fall_value: 0.0,
            landmark_free_fall: false,
            landmark_noise_time: 0.0,
            old_velocity: Vec3::default(),
            old_view_angles: Vec3::default(),
            killer_yaw: 0.0,
            buttons: 0,
            latched_buttons: 0,
            weapon_thunk: false,
            bob_time: 0.0,
            bob_move: 0.0,
            event: String::new(),
            animation_priority: 0,
            animation_end: 39,
            animation_duck: false,
            animation_run: false,
            loop_sound: String::new(),
            selected_item: Some("q2:weapon_blaster".to_string()),
            show_scores: false,
            show_inventory: false,
            show_help: false,
            chase_target: None,
            coop_respawn: None,
            flood_times: Vec::new(),
            flood_lock_until: 0.0,
        }
    }
}

/// Character weapon observation (`Q2CharacterWeapon`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CharacterWeapon {
    /// Q2 weapon name.
    pub q2_name: Option<Q2WeaponName>,
    /// Ammo item.
    pub ammo: Option<ItemId>,
    /// Kick angles.
    pub kick_angles: Vec3,
    /// Kick origin.
    pub kick_origin: Vec3,
    /// Loop sound.
    pub loop_sound: String,
}

/// Body changes (`move` changes).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Q2BodyChanges {
    /// Origin.
    pub origin: Option<Vec3>,
    /// Angles.
    pub angles: Option<Vec3>,
    /// Velocity.
    pub velocity: Option<Vec3>,
    /// Bounds.
    pub bounds: Option<Bounds>,
    /// Ground.
    pub ground: Option<Option<ActorId>>,
}

/// Narrow character context (`Q2CharacterContext`).
///
/// Implemented by the full player context and by the standalone
/// character actor so environment and view logic is shared.
pub trait Q2CharacterContext {
    /// Actor id.
    fn actor_id(&self) -> ActorId;
    /// Owned actor.
    fn owned_actor(&self) -> OwnedActor;
    /// Current time.
    fn now(&mut self) -> f64;
    /// Random draw.
    fn random(&mut self) -> f64;
    /// Movement observation.
    fn movement(&mut self) -> Q2PlayerMovement;
    /// Rules.
    fn rules(&self) -> Q2PlayerRules;
    /// Powerups.
    fn powerups(&mut self) -> Q2PlayerPowerups;
    /// Weapon observation.
    fn weapon_state(&mut self) -> Option<Q2CharacterWeapon>;
    /// Apply environment damage.
    fn environment_damage(&mut self, amount: f64, means: i32, flags: i32);
    /// Emit noise.
    fn noise(&mut self, origin: Vec3);
    /// Read the body.
    fn body(&mut self) -> BodyState;
    /// Move the body.
    fn move_body(&mut self, changes: Q2BodyChanges, link: bool);
    /// Play a sound.
    fn sound(&mut self, path: &str, channel: i32, volume: f64, attenuation: f64);
    /// Emit a presentation event.
    fn emit(&mut self, event: Q2PresentationEvent);
    /// Read point contents.
    fn point_contents(&mut self, point: Vec3) -> i32;
    /// Read combat state.
    fn combat(&mut self) -> Option<CombatState>;
    /// Count an inventory item.
    fn inventory_count(&mut self, item: &ItemId) -> f64;
    /// Game mode.
    fn mode(&self) -> Q2Mode;
    /// Deathmatch flags.
    fn deathmatch_flags(&self) -> i32;
    /// Edition.
    fn edition(&self) -> Q2Edition;
    /// Read a player state snapshot.
    fn state_snapshot(&mut self) -> Q2PlayerState;
    /// Mutate the player state.
    fn with_state<R>(&mut self, f: impl FnOnce(&mut Q2PlayerState) -> R) -> R;
    /// Read an entity snapshot.
    fn entity_snapshot(&mut self) -> crate::q2::foundation::host::Q2Entity;
    /// Mutate the entity.
    fn with_entity<R>(&mut self, f: impl FnOnce(&mut crate::q2::foundation::host::Q2Entity) -> R) -> R;
}

/// Full player context (`Q2PlayerContext`).
///
/// Transient facade over an admitted player and the game services; also
/// implements the narrow character context for shared environment and
/// view logic.
pub struct Q2PlayerContext<'game> {
    /// Player actor.
    pub actor: ActorId,
    /// Game services.
    pub game: &'game mut Q2GameServices,
}
