//! Quake III base/game: state.
//!
//! Donor provenance: `src/content/q3/base/game/state.ts`.

use qa_core::identity::ActorId;
use qa_core::math::angle_normalize180;
use qa_core::math::vec3;
use qa_core::math::vector_to_angles;
use qa_core::math::Bounds;
use qa_core::math::Vec3;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::entities::ItemDefinition;
use crate::q3::base::game::hitscan::{Q3BulletAttack, RailStatistics, RailStatisticsOutcome};
use crate::q3::base::game::items_core::Q3GameItemsError;
use crate::q3::base::game::mover::MoverActorAccess;
use crate::q3::base::game::rankings::Q3RankingReports;
use crate::q3::base::game::save_callbacks::Q3CallbackCatalog;
use crate::q3::base::game::utilities::GameUtilityScratch;
use crate::q3::base::game::weapon::{BulletHost, ContactHost, RailHost, ShotgunHost};
use crate::q3::base::mirrors::Q3BaseError;
use crate::q3::base::shared::definitions::{
    stat_schema, weapon_count, EntityEvent, MoveType, Powerup, Product, StatSchema, Team, Weapon,
};
use crate::q3::base::shared::entity_shared::EntityCollisionModel;
use crate::q3::base::shared::entity_state::EntityState;
use crate::q3::base::shared::trajectory::{Trajectory, TrajectoryType};
use crate::q3::base::world::{ActorTraceQuery, ActorTraceResult, LinkState};
use crate::value::ValueError;

// ---------------------------------------------------------------------------
// state.ts: game state (`src/content/q3/base/game/state.ts`, g_local.h)
// ---------------------------------------------------------------------------

/// Maximum clients (`MAX_CLIENTS`).
pub const MAX_CLIENTS: usize = 64;

/// Maximum entities (`MAX_GENTITIES`).
pub const MAX_GENTITIES: usize = 1024;

/// Connection state (`ConnectionState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ConnectionState {
    /// Disconnected.
    Disconnected = 0,
    /// Connecting.
    Connecting = 1,
    /// Connected.
    Connected = 2,
}

impl ConnectionState {
    /// Convert a stored integer, rejecting unknown values.
    pub fn from_i32(value: i32) -> Result<Self, Q3GameError> {
        match value {
            0 => Ok(Self::Disconnected),
            1 => Ok(Self::Connecting),
            2 => Ok(Self::Connected),
            _ => Err(failure(format!("unknown connection state {value}"))),
        }
    }
}

/// Spectator state (`SpectatorState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum SpectatorState {
    /// Not spectating.
    Not = 0,
    /// Free.
    Free = 1,
    /// Follow.
    Follow = 2,
    /// Scoreboard.
    Scoreboard = 3,
}

/// Team state (`TeamState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum TeamState {
    /// Begin.
    Begin = 0,
    /// Active.
    Active = 1,
}

/// Mover state (`MoverState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum MoverState {
    /// At position 1.
    Pos1 = 0,
    /// At position 2.
    Pos2 = 1,
    /// Moving 1 to 2.
    OneToTwo = 2,
    /// Moving 2 to 1.
    TwoToOne = 3,
}

impl MoverState {
    /// Convert a stored integer, rejecting unknown values.
    pub fn from_i32(value: i32) -> Result<Self, Q3GameError> {
        match value {
            0 => Ok(Self::Pos1),
            1 => Ok(Self::Pos2),
            2 => Ok(Self::OneToTwo),
            3 => Ok(Self::TwoToOne),
            _ => Err(failure(format!("Invalid mover state {value}"))),
        }
    }
}

/// Game flags (`GameFlags`).
pub struct GameFlags;

impl GameFlags {
    /// God mode.
    pub const GODMODE: i32 = 0x10;
    /// Notarget.
    pub const NOTARGET: i32 = 0x20;
    /// Team slave.
    pub const TEAMSLAVE: i32 = 0x400;
    /// No knockback.
    pub const NO_KNOCKBACK: i32 = 0x800;
    /// Dropped item.
    pub const DROPPED_ITEM: i32 = 0x1000;
    /// No bots.
    pub const NO_BOTS: i32 = 0x2000;
    /// No humans.
    pub const NO_HUMANS: i32 = 0x4000;
    /// Force gesture.
    pub const FORCE_GESTURE: i32 = 0x8000;
}

/// Player team state (`PlayerTeamState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerTeamState {
    /// State.
    pub state: i32,
    /// Location.
    pub location: i32,
    /// Captures.
    pub captures: i32,
    /// Base defense.
    pub base_defense: i32,
    /// Carrier defense.
    pub carrier_defense: i32,
    /// Flag recovery.
    pub flag_recovery: i32,
    /// Frag carrier.
    pub frag_carrier: i32,
    /// Assists.
    pub assists: i32,
    /// Last hurt carrier.
    pub last_hurt_carrier: i32,
    /// Last returned flag.
    pub last_returned_flag: i32,
    /// Flag since.
    pub flag_since: i32,
    /// Last fragged carrier.
    pub last_fragged_carrier: i32,
}

impl Default for PlayerTeamState {
    fn default() -> Self {
        Self {
            state: TeamState::Begin as i32,
            location: 0,
            captures: 0,
            base_defense: 0,
            carrier_defense: 0,
            flag_recovery: 0,
            frag_carrier: 0,
            assists: 0,
            last_hurt_carrier: 0,
            last_returned_flag: 0,
            flag_since: 0,
            last_fragged_carrier: 0,
        }
    }
}

/// Client session (`ClientSession`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientSession {
    /// Session team.
    pub session_team: i32,
    /// Spectator time.
    pub spectator_time: i32,
    /// Spectator state.
    pub spectator_state: i32,
    /// Spectator client.
    pub spectator_client: i32,
    /// Wins.
    pub wins: i32,
    /// Losses.
    pub losses: i32,
    /// Team leader.
    pub team_leader: i32,
}

impl Default for ClientSession {
    fn default() -> Self {
        Self {
            session_team: Team::TeamFree as i32,
            spectator_time: 0,
            spectator_state: SpectatorState::Not as i32,
            spectator_client: 0,
            wins: 0,
            losses: 0,
            team_leader: 0,
        }
    }
}

/// Persistant client data (`ClientPersistant`).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientPersistant {
    /// Connected.
    pub connected: i32,
    /// Command.
    pub cmd: Q3UserCommand,
    /// Local client.
    pub local_client: bool,
    /// Initial spawn.
    pub initial_spawn: bool,
    /// Predict item pickup.
    pub predict_item_pickup: bool,
    /// Pmove fixed.
    pub pmove_fixed: bool,
    /// Net name.
    pub netname: String,
    /// Max health.
    pub max_health: i32,
    /// Enter time.
    pub enter_time: i32,
    /// Team state.
    pub team_state: PlayerTeamState,
    /// Vote count.
    pub vote_count: i32,
    /// Team vote count.
    pub team_vote_count: i32,
    /// Team info.
    pub team_info: bool,
}

impl Default for ClientPersistant {
    fn default() -> Self {
        Self {
            connected: ConnectionState::Disconnected as i32,
            cmd: Q3UserCommand::default(),
            local_client: false,
            initial_spawn: false,
            predict_item_pickup: false,
            pmove_fixed: false,
            netname: String::new(),
            max_health: 0,
            enter_time: 0,
            team_state: PlayerTeamState::default(),
            vote_count: 0,
            team_vote_count: 0,
            team_info: false,
        }
    }
}

/// Damage/use participant (`DamageParticipant` / `UseParticipant` /
/// `DamageInflictor`): a native entity slot or a shared actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Participant {
    /// Native entity slot.
    Entity(usize),
    /// Shared actor.
    SharedActor(ActorId),
}

/// Think callback (`EntityThink`).
pub type EntityThink = Rc<dyn Fn(&mut dyn Q3Driver, usize)>;

/// Blocked callback (`EntityBlocked`).
pub type EntityBlocked = Rc<dyn Fn(&mut dyn Q3Driver, usize, &Participant)>;

/// Touch callback (`EntityTouch`).
pub type EntityTouch = Rc<dyn Fn(&mut dyn Q3Driver, usize, &Participant, &TouchContact)>;

/// Use callback (`EntityUse`).
pub type EntityUse = Rc<dyn Fn(&mut dyn Q3Driver, usize, Option<&Participant>, Option<&Participant>)>;

/// Pain callback (`EntityPain`).
pub type EntityPain = Rc<dyn Fn(&mut dyn Q3Driver, usize, &Participant, i32)>;

/// Die callback (`EntityDie`).
pub type EntityDie = Rc<dyn Fn(&mut dyn Q3Driver, usize, &Participant, &Participant, i32, i32)>;

/// Entity classname binding (`GameEntity` classname state).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassName {
    /// Plain value.
    Value(Option<String>),
    /// Bound to a client netname.
    ClientName(usize),
}

/// Game client (`GameClient`, source-zero gclient_t).
#[derive(Debug, Clone, PartialEq)]
pub struct GameClient {
    /// Player state.
    pub ps: Q3PlayerState,
    /// Persistant data.
    pub pers: ClientPersistant,
    /// Session.
    pub sess: ClientSession,
    /// Ready to exit.
    pub ready_to_exit: bool,
    /// Noclip.
    pub noclip: bool,
    /// Last command time.
    pub last_cmd_time: i32,
    /// Buttons.
    pub buttons: i32,
    /// Old buttons.
    pub old_buttons: i32,
    /// Latched buttons.
    pub latched_buttons: i32,
    /// Old origin.
    pub old_origin: Vec3,
    /// Damage armor.
    pub damage_armor: i32,
    /// Damage blood.
    pub damage_blood: i32,
    /// Damage knockback.
    pub damage_knockback: i32,
    /// Damage from.
    pub damage_from: Vec3,
    /// Damage from world.
    pub damage_from_world: bool,
    /// Accurate count.
    pub accurate_count: i32,
    /// Accuracy shots.
    pub accuracy_shots: i32,
    /// Accuracy hits.
    pub accuracy_hits: i32,
    /// Last killed client.
    pub last_killed_client: i32,
    /// Last hurt client.
    pub last_hurt_client: i32,
    /// Last hurt means of death.
    pub last_hurt_mod: i32,
    /// Respawn time.
    pub respawn_time: i32,
    /// Inactivity time.
    pub inactivity_time: i32,
    /// Inactivity warning.
    pub inactivity_warning: bool,
    /// Reward time.
    pub reward_time: i32,
    /// Air out time.
    pub air_out_time: i32,
    /// Last kill time.
    pub last_kill_time: i32,
    /// Fire held.
    pub fire_held: bool,
    /// Hook entity slot.
    pub hook: Option<usize>,
    /// Switch team time.
    pub switch_team_time: i32,
    /// Time residual.
    pub time_residual: i32,
    /// Persistant powerup entity slot.
    pub persistant_powerup: Option<usize>,
    /// Portal identifier.
    pub portal_id: i32,
    /// Ammo times.
    pub ammo_times: Q3PlayerSlots,
    /// Invulnerability time.
    pub invulnerability_time: i32,
    /// Area bits.
    pub areabits: Option<Vec<u8>>,
}

impl GameClient {
    /// Source-zero client for a product.
    #[must_use]
    pub fn new(product: Product) -> Self {
        Self {
            ps: Q3PlayerState::new(product),
            pers: ClientPersistant::default(),
            sess: ClientSession::default(),
            ready_to_exit: false,
            noclip: false,
            last_cmd_time: 0,
            buttons: 0,
            old_buttons: 0,
            latched_buttons: 0,
            old_origin: vec3(0.0, 0.0, 0.0),
            damage_armor: 0,
            damage_blood: 0,
            damage_knockback: 0,
            damage_from: vec3(0.0, 0.0, 0.0),
            damage_from_world: false,
            accurate_count: 0,
            accuracy_shots: 0,
            accuracy_hits: 0,
            last_killed_client: 0,
            last_hurt_client: 0,
            last_hurt_mod: 0,
            respawn_time: 0,
            inactivity_time: 0,
            inactivity_warning: false,
            reward_time: 0,
            air_out_time: 0,
            last_kill_time: 0,
            fire_held: false,
            hook: None,
            switch_team_time: 0,
            time_residual: 0,
            persistant_powerup: None,
            portal_id: 0,
            ammo_times: Q3PlayerSlots::new(weapon_count(product) as usize),
            invulnerability_time: 0,
            areabits: None,
        }
    }
}

/// Game entity (`GameEntity`, source-zero gentity_t with slot metadata).
#[derive(Clone)]
pub struct GameEntity {
    /// Entity slot.
    pub slot: usize,
    /// In-use flag (`binding.active()` projection).
    pub inuse: bool,
    /// Actor handle.
    pub actor: ActorId,
    /// Network state.
    pub s: EntityState,
    /// Shared fields.
    pub r: EntitySharedFields,
    /// Client slot.
    pub client: Option<usize>,
    classname: ClassName,
    /// Spawn flags.
    pub spawnflags: i32,
    /// Never free.
    pub never_free: bool,
    /// Flags.
    pub flags: i32,
    /// Model.
    pub model: Option<String>,
    /// Model 2.
    pub model2: Option<String>,
    /// Free time.
    pub freetime: i32,
    /// Event time.
    pub event_time: i32,
    /// Free after event.
    pub free_after_event: bool,
    /// Unlink after event.
    pub unlink_after_event: bool,
    /// Physics object.
    pub physics_object: bool,
    /// Physics bounce.
    pub physics_bounce: i32,
    /// Clip mask.
    pub clipmask: i32,
    /// Mover state.
    pub mover_state: i32,
    /// Sound position 1.
    pub sound_pos1: i32,
    /// Sound 1 to 2.
    pub sound1to2: i32,
    /// Sound 2 to 1.
    pub sound2to1: i32,
    /// Sound position 2.
    pub sound_pos2: i32,
    /// Sound loop.
    pub sound_loop: i32,
    /// Parent slot.
    pub parent: Option<usize>,
    /// Next train slot.
    pub next_train: Option<usize>,
    /// Previous train slot.
    pub prev_train: Option<usize>,
    /// Position 1.
    pub pos1: Vec3,
    /// Position 2.
    pub pos2: Vec3,
    /// Message.
    pub message: Option<String>,
    /// Timestamp.
    pub timestamp: i32,
    /// Angle.
    pub angle: f32,
    /// Target.
    pub target: Option<String>,
    /// Target name.
    pub targetname: Option<String>,
    /// Team.
    pub team: Option<String>,
    /// Target shader name.
    pub target_shader_name: Option<String>,
    /// Target shader new name.
    pub target_shader_new_name: Option<String>,
    /// Target entity slot.
    pub target_ent: Option<usize>,
    /// Speed.
    pub speed: f32,
    /// Move direction.
    pub movedir: Vec3,
    /// Next think time (raw field; scheduling goes through the pool hook).
    pub nextthink: i32,
    /// Think callback.
    pub think: Option<EntityThink>,
    /// Reached callback.
    pub reached: Option<EntityThink>,
    /// Blocked callback.
    pub blocked: Option<EntityBlocked>,
    /// Touch callback.
    pub touch: Option<EntityTouch>,
    /// Use callback.
    pub use_callback: Option<EntityUse>,
    /// Pain callback.
    pub pain: Option<EntityPain>,
    /// Die callback.
    pub die: Option<EntityDie>,
    /// Pain debounce time.
    pub pain_debounce_time: i32,
    /// Fly sound debounce time.
    pub fly_sound_debounce_time: i32,
    /// Last move time.
    pub last_move_time: i32,
    /// Health (`binding.health()` projection).
    pub health: i32,
    /// Takes damage (`binding.takedamage()` projection).
    pub takedamage: bool,
    /// Damage.
    pub damage: i32,
    /// Splash damage.
    pub splash_damage: i32,
    /// Splash radius.
    pub splash_radius: i32,
    /// Means of death.
    pub method_of_death: i32,
    /// Splash means of death.
    pub splash_method_of_death: i32,
    /// Count.
    pub count: i32,
    /// Chain slot.
    pub chain: Option<usize>,
    /// Enemy slot.
    pub enemy: Option<usize>,
    /// Activator slot.
    pub activator: Option<usize>,
    /// Activation participant.
    pub activation: Option<Participant>,
    /// Team chain slot.
    pub teamchain: Option<usize>,
    /// Team master slot.
    pub teammaster: Option<usize>,
    /// Kamikaze time.
    pub kamikaze_time: i32,
    /// Kamikaze shock time.
    pub kamikaze_shock_time: i32,
    /// Water type.
    pub watertype: i32,
    /// Water level.
    pub waterlevel: i32,
    /// Noise index.
    pub noise_index: i32,
    /// Wait.
    pub wait: f32,
    /// Random.
    pub random: f32,
    /// Item table index.
    pub item: Option<usize>,
}

impl GameEntity {
    /// Source-zero entity in a slot with an actor handle.
    pub fn new(slot: usize, actor: ActorId) -> Result<Self, Q3GameError> {
        if slot >= MAX_GENTITIES {
            return Err(range("Game entity slot outside 0..1023"));
        }
        Ok(Self {
            slot,
            inuse: false,
            actor,
            s: EntityState::default(),
            r: EntitySharedFields::default(),
            client: None,
            classname: ClassName::Value(None),
            spawnflags: 0,
            never_free: false,
            flags: 0,
            model: None,
            model2: None,
            freetime: 0,
            event_time: 0,
            free_after_event: false,
            unlink_after_event: false,
            physics_object: false,
            physics_bounce: 0,
            clipmask: 0,
            mover_state: MoverState::Pos1 as i32,
            sound_pos1: 0,
            sound1to2: 0,
            sound2to1: 0,
            sound_pos2: 0,
            sound_loop: 0,
            parent: None,
            next_train: None,
            prev_train: None,
            pos1: vec3(0.0, 0.0, 0.0),
            pos2: vec3(0.0, 0.0, 0.0),
            message: None,
            timestamp: 0,
            angle: 0.0,
            target: None,
            targetname: None,
            team: None,
            target_shader_name: None,
            target_shader_new_name: None,
            target_ent: None,
            speed: 0.0,
            movedir: vec3(0.0, 0.0, 0.0),
            nextthink: 0,
            think: None,
            reached: None,
            blocked: None,
            touch: None,
            use_callback: None,
            pain: None,
            die: None,
            pain_debounce_time: 0,
            fly_sound_debounce_time: 0,
            last_move_time: 0,
            health: 0,
            takedamage: false,
            damage: 0,
            splash_damage: 0,
            splash_radius: 0,
            method_of_death: 0,
            splash_method_of_death: 0,
            count: 0,
            chain: None,
            enemy: None,
            activator: None,
            activation: None,
            teamchain: None,
            teammaster: None,
            kamikaze_time: 0,
            kamikaze_shock_time: 0,
            watertype: 0,
            waterlevel: 0,
            noise_index: 0,
            wait: 0.0,
            random: 0.0,
            item: None,
        })
    }

    /// Read the classname (`get classname()`; client-bound names need the pool).
    #[must_use]
    pub fn classname_value(&self) -> Option<&str> {
        match &self.classname {
            ClassName::Value(value) => value.as_deref(),
            ClassName::ClientName(_) => None,
        }
    }

    /// Read the classname state.
    #[must_use]
    pub fn classname_state(&self) -> &ClassName {
        &self.classname
    }

    /// Write the classname (`set classname()`).
    pub fn set_classname(&mut self, value: Option<String>) {
        self.classname = ClassName::Value(value);
    }

    /// Bind the classname to a client netname (`bindClientName`).
    pub fn bind_client_name(&mut self, client: usize) {
        self.classname = ClassName::ClientName(client);
    }

    /// Capture the classname binding (`captureClassname`).
    #[must_use]
    pub fn capture_classname(&self) -> ClassName {
        self.classname.clone()
    }

    /// Resolve a possibly client-bound classname through client netnames.
    #[must_use]
    pub fn resolve_classname(&self, netname: Option<&str>) -> Option<String> {
        match &self.classname {
            ClassName::Value(value) => value.clone(),
            ClassName::ClientName(_) => netname.map(str::to_string),
        }
    }
}

impl std::fmt::Debug for GameEntity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameEntity")
            .field("slot", &self.slot)
            .field("inuse", &self.inuse)
            .field("actor", &self.actor)
            .field("s", &self.s)
            .field("r", &self.r)
            .field("client", &self.client)
            .field("classname", &self.classname)
            .field("health", &self.health)
            .field("takedamage", &self.takedamage)
            .field("think", &self.think.is_some())
            .field("reached", &self.reached.is_some())
            .field("blocked", &self.blocked.is_some())
            .field("touch", &self.touch.is_some())
            .field("use_callback", &self.use_callback.is_some())
            .field("pain", &self.pain.is_some())
            .field("die", &self.die.is_some())
            .finish_non_exhaustive()
    }
}

/// Create a game client (`createGameClient`).
#[must_use]
pub fn create_game_client(product: Product) -> GameClient {
    GameClient::new(product)
}

/// Create a game entity (`createGameEntity`).
pub fn create_game_entity(slot: usize, actor: ActorId) -> Result<GameEntity, Q3GameError> {
    GameEntity::new(slot, actor)
}

// ---------------------------------------------------------------------------
// Unified from `mirrors_game_state.rs` (hoist: q3 state mirror).
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Game failures (`Error`, `RangeError`, `CommonError("drop")`, `TextParseError`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q3GameError {
    /// Fatal drop (`CommonError("drop", ...)`); spawn dispatch must not free on this path.
    Drop(String),
    /// Out-of-range value (`RangeError`).
    Range(String),
    /// Spawn text parse failure (`TextParseError`).
    Parse {
        /// Source name.
        source: String,
        /// 1-based line.
        line: usize,
        /// 1-based column.
        column: usize,
        /// Message.
        message: String,
    },
    /// Ordinary failure (`Error`).
    Failure(String),
}

impl std::fmt::Display for Q3GameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Drop(message) | Self::Range(message) | Self::Failure(message) => write!(f, "{message}"),
            Self::Parse {
                source,
                line,
                column,
                message,
            } => {
                write!(f, "{source}:{line}:{column}: {message}")
            }
        }
    }
}

impl std::error::Error for Q3GameError {}

impl From<ValueError> for Q3GameError {
    fn from(value: ValueError) -> Self {
        Self::Failure(value.0)
    }
}

impl From<Q3GameItemsError> for Q3GameError {
    fn from(value: Q3GameItemsError) -> Self {
        match value {
            Q3GameItemsError::Drop(message) => Self::Drop(message),
            Q3GameItemsError::Range(message) => Self::Range(message),
            Q3GameItemsError::Invalid(message) | Q3GameItemsError::Missing(message) => Self::Failure(message),
        }
    }
}

impl From<Q3BaseError> for Q3GameError {
    fn from(value: Q3BaseError) -> Self {
        match value {
            Q3BaseError::Drop(message) => Self::Drop(message),
            Q3BaseError::Range(message) => Self::Range(message),
            Q3BaseError::Invalid(message) => Self::Failure(message),
        }
    }
}

pub(crate) fn failure(message: impl Into<String>) -> Q3GameError {
    Q3GameError::Failure(message.into())
}

pub(crate) fn range(message: impl Into<String>) -> Q3GameError {
    Q3GameError::Range(message.into())
}

/// Validate a Latin-1 byte string (`checkByteString` / `byteString` donors).
pub(crate) fn latin1_bytes(value: &str) -> Result<Vec<u8>, Q3GameError> {
    let mut bytes = Vec::with_capacity(value.len());
    for ch in value.chars() {
        let code = ch as u32;
        if code == 0 || code > 255 {
            return Err(range("Spawn strings must contain non-NUL byte characters"));
        }
        bytes.push(code as u8);
    }
    Ok(bytes)
}

/// Rebuild text from Latin-1 bytes.
pub(crate) fn latin1_string(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| char::from_u32(u32::from(*byte)).unwrap_or('\u{FFFD}'))
        .collect()
}

/// Fold ASCII uppercase to lowercase without touching other bytes.
pub(crate) fn ascii_lower(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().map(|byte| byte.to_ascii_lowercase()).collect()
}

// (donor `src/contracts/world.ts`, `src/content/q3/base/world.ts`,
// `src/content/q3/base/shared/entity-shared.ts`)
// ---------------------------------------------------------------------------

/// Shared body state (`BodyState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3BodyState {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Bounds.
    pub bounds: Bounds,
    /// Support actor.
    pub ground: Option<ActorId>,
}

/// Linked body snapshot (`LinkedBody`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3LinkedBody {
    /// Actor.
    pub actor: ActorId,
    /// State.
    pub state: Q3BodyState,
    /// Absolute bounds.
    pub absolute_bounds: Bounds,
    /// Link count.
    pub link_count: i32,
}

/// Touch surface (`TouchContact["surface"]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3TouchSurface {
    /// Name.
    pub name: String,
    /// Native flags.
    pub native_flags: i32,
    /// Native value.
    pub native_value: i32,
}

/// Touch contact (`TouchContact`).
#[derive(Debug, Clone, PartialEq)]
pub struct TouchContact {
    /// Self actor.
    pub self_actor: ActorId,
    /// Other actor.
    pub other: ActorId,
    /// Contact plane.
    pub plane: Option<qa_core::math::Plane>,
    /// Surface.
    pub surface: Option<Q3TouchSurface>,
}

/// Shared entity fields (`EntityShared`, flattened to plain data).
#[derive(Debug, Clone, PartialEq)]
pub struct EntitySharedFields {
    /// Server flags.
    pub sv_flags: i32,
    /// Single client.
    pub single_client: i32,
    /// Collision model.
    pub model: EntityCollisionModel,
    /// Contents.
    pub contents: i32,
    /// Owner number.
    pub owner_num: i32,
    /// Mins.
    pub mins: Vec3,
    /// Maxs.
    pub maxs: Vec3,
    /// Current origin.
    pub current_origin: Vec3,
    /// Current angles.
    pub current_angles: Vec3,
    /// Linked bounds, when linked.
    pub linked_bounds: Option<Bounds>,
    /// Linked flag.
    pub linked: bool,
    /// Absolute-min override (`absmin` setter).
    pub absmin_override: Option<Vec3>,
    /// Absolute-max override (`absmax` setter).
    pub absmax_override: Option<Vec3>,
    /// Previous link snapshot.
    pub previous_link: Option<Q3LinkedBody>,
    /// Support actor (`binding.body` ground projection).
    pub ground: Option<ActorId>,
}

impl Default for EntitySharedFields {
    fn default() -> Self {
        Self {
            sv_flags: 0,
            single_client: 0,
            model: EntityCollisionModel::Box,
            contents: 0,
            owner_num: 0,
            mins: vec3(0.0, 0.0, 0.0),
            maxs: vec3(0.0, 0.0, 0.0),
            current_origin: vec3(0.0, 0.0, 0.0),
            current_angles: vec3(0.0, 0.0, 0.0),
            linked_bounds: None,
            linked: false,
            absmin_override: None,
            absmax_override: None,
            previous_link: None,
            ground: None,
        }
    }
}

impl EntitySharedFields {
    /// Absolute mins (`absmin` getter chain).
    #[must_use]
    pub fn absmin(&self) -> Vec3 {
        self.absmin_override
            .or_else(|| self.linked_bounds.map(|bounds| bounds.min))
            .or_else(|| self.previous_link.as_ref().map(|link| link.absolute_bounds.min))
            .unwrap_or_else(|| vec3(0.0, 0.0, 0.0))
    }

    /// Absolute maxs (`absmax` getter chain).
    #[must_use]
    pub fn absmax(&self) -> Vec3 {
        self.absmax_override
            .or_else(|| self.linked_bounds.map(|bounds| bounds.max))
            .or_else(|| self.previous_link.as_ref().map(|link| link.absolute_bounds.max))
            .unwrap_or_else(|| vec3(0.0, 0.0, 0.0))
    }

    /// Body state projection (`binding.body.read()`).
    #[must_use]
    pub fn body_state(&self, velocity: Vec3) -> Q3BodyState {
        Q3BodyState {
            origin: self.current_origin,
            angles: self.current_angles,
            velocity,
            bounds: Bounds {
                min: self.mins,
                max: self.maxs,
            },
            ground: self.ground.clone(),
        }
    }
}

/// Player slot storage (`PlayerStateSlots`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3PlayerSlots {
    values: Vec<i32>,
}

impl Q3PlayerSlots {
    /// Zeroed slots.
    #[must_use]
    pub fn new(length: usize) -> Self {
        Self {
            values: vec![0; length],
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
    #[must_use]
    pub fn get(&self, index: usize) -> i32 {
        self.values[index]
    }

    /// Write a slot.
    pub fn set(&mut self, index: usize, value: i32) {
        self.values[index] = value;
    }

    /// Copy all slots (`copy()`).
    #[must_use]
    pub fn copy_vec(&self) -> Vec<i32> {
        self.values.clone()
    }

    /// Restore all slots, requiring an exact count (`restoreSlots`).
    pub fn restore(&mut self, values: &[i32]) -> Result<(), Q3GameError> {
        if values.len() != self.values.len() {
            return Err(failure("Q3 saved player slot count mismatch"));
        }
        self.values.copy_from_slice(values);
        Ok(())
    }
}

/// User command (`UserCommand`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3UserCommand {
    /// Server time.
    pub server_time: i32,
    /// Angles.
    pub angles: Vec3,
    /// Buttons.
    pub buttons: i32,
    /// Weapon.
    pub weapon: i32,
    /// Forward move.
    pub forwardmove: i32,
    /// Right move.
    pub rightmove: i32,
    /// Up move.
    pub upmove: i32,
}

impl Default for Q3UserCommand {
    fn default() -> Self {
        Self {
            server_time: 0,
            angles: vec3(0.0, 0.0, 0.0),
            buttons: 0,
            weapon: Weapon::WpNone as i32,
            forwardmove: 0,
            rightmove: 0,
            upmove: 0,
        }
    }
}

/// Player state (`PlayerState`, flattened to plain data).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3PlayerState {
    /// Product.
    pub product: Product,
    /// Command time.
    pub command_time: i32,
    /// Movement type.
    pub pm_type: i32,
    /// Bob cycle.
    pub bob_cycle: i32,
    /// Move flags.
    pub pm_flags: i32,
    /// Move time.
    pub pm_time: i32,
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Weapon time.
    pub weapon_time: i32,
    /// Gravity.
    pub gravity: i32,
    /// Speed.
    pub speed: i32,
    /// Delta angles.
    pub delta_angles: [i32; 3],
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
    /// Events ring.
    pub events: Q3PlayerSlots,
    /// Event parameters ring.
    pub event_parms: Q3PlayerSlots,
    /// External event.
    pub external_event: i32,
    /// External event parameter.
    pub external_event_parm: i32,
    /// External event time.
    pub external_event_time: i32,
    /// Client number.
    pub client_num: i32,
    /// Weapon.
    pub weapon: i32,
    /// Weapon state.
    pub weapon_state: i32,
    /// View angles.
    pub viewangles: Vec3,
    /// View height.
    pub viewheight: f32,
    /// Damage event.
    pub damage_event: i32,
    /// Damage yaw.
    pub damage_yaw: i32,
    /// Damage pitch.
    pub damage_pitch: i32,
    /// Damage count.
    pub damage_count: i32,
    /// Stats.
    pub stats: Q3PlayerSlots,
    /// Persistant.
    pub persistant: Q3PlayerSlots,
    /// Powerups.
    pub powerups: Q3PlayerSlots,
    /// Ammo.
    pub ammo: Q3PlayerSlots,
    /// Generic 1.
    pub generic1: i32,
    /// Loop sound.
    pub loop_sound: i32,
    /// Jump pad entity.
    pub jumppad_ent: i32,
    /// Ping.
    pub ping: i32,
    /// Pmove frame count.
    pub pmove_framecount: i32,
    /// Jump pad frame.
    pub jumppad_frame: i32,
    /// Entity event sequence.
    pub entity_event_sequence: i32,
}

impl Q3PlayerState {
    /// Zeroed player state for a product (`createPlayerState`).
    #[must_use]
    pub fn new(product: Product) -> Self {
        Self {
            product,
            command_time: 0,
            pm_type: 0,
            bob_cycle: 0,
            pm_flags: 0,
            pm_time: 0,
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            weapon_time: 0,
            gravity: 0,
            speed: 0,
            delta_angles: [0, 0, 0],
            ground_entity_num: 0,
            legs_timer: 0,
            legs_anim: 0,
            torso_timer: 0,
            torso_anim: 0,
            movement_dir: 0,
            grapple_point: vec3(0.0, 0.0, 0.0),
            e_flags: 0,
            event_sequence: 0,
            events: Q3PlayerSlots::new(2),
            event_parms: Q3PlayerSlots::new(2),
            external_event: 0,
            external_event_parm: 0,
            external_event_time: 0,
            client_num: 0,
            weapon: 0,
            weapon_state: 0,
            viewangles: vec3(0.0, 0.0, 0.0),
            viewheight: 0.0,
            damage_event: 0,
            damage_yaw: 0,
            damage_pitch: 0,
            damage_count: 0,
            stats: Q3PlayerSlots::new(16),
            persistant: Q3PlayerSlots::new(16),
            powerups: Q3PlayerSlots::new(16),
            ammo: Q3PlayerSlots::new(16),
            generic1: 0,
            loop_sound: 0,
            jumppad_ent: 0,
            ping: 0,
            pmove_framecount: 0,
            jumppad_frame: 0,
            entity_event_sequence: 0,
        }
    }

    /// Health stat (`get health()`).
    #[must_use]
    pub fn health(&self) -> i32 {
        let slot = match stat_schema(self.product) {
            StatSchema::Base(layout) => layout.health,
            StatSchema::Missionpack(layout) => layout.health,
        } as usize;
        self.stats.get(slot)
    }

    /// Write the health stat (`set health()`).
    pub fn set_health(&mut self, value: i32) {
        let slot = match stat_schema(self.product) {
            StatSchema::Base(layout) => layout.health,
            StatSchema::Missionpack(layout) => layout.health,
        } as usize;
        self.stats.set(slot, value);
    }

    /// Queue a predictable event (`addEvent`).
    pub fn add_event(&mut self, event: i32, parameter: i32) {
        let sequence = self.event_sequence;
        let index = (sequence & 1) as usize;
        self.events.set(index, event);
        self.event_parms.set(index, parameter);
        self.event_sequence = sequence.wrapping_add(1);
    }
}

/// Entity pool surface used by this module (`EntityPool`).
pub trait EntityPool {
    /// Product.
    fn product(&self) -> Product;
    /// Active entity count.
    fn num_entities(&self) -> usize;
    /// Maximum clients.
    fn max_clients(&self) -> usize;
    /// Borrow an entity.
    fn entity(&self, slot: usize) -> Option<&GameEntity>;
    /// Mutably borrow an entity.
    fn entity_mut(&mut self, slot: usize) -> Option<&mut GameEntity>;
    /// Borrow a client.
    fn client(&self, slot: usize) -> Option<&GameClient>;
    /// Mutably borrow a client.
    fn client_mut(&mut self, slot: usize) -> Option<&mut GameClient>;
    /// Spawn an entity, returning its slot (`spawn`).
    fn spawn_entity(&mut self) -> Result<usize, Q3GameError>;
    /// Free an entity (`free`).
    fn free_entity(&mut self, slot: usize);
    /// Allocate a temporary event entity (`tempEntity`).
    fn temp_entity(&mut self, origin: Vec3, event: EntityEvent) -> usize;
    /// Add an entity event (`addEvent`).
    fn add_event(&mut self, slot: usize, event: EntityEvent, parm: i32);
    /// Set nextthink and schedule it (`nextthink` setter + `binding.schedule`).
    fn set_nextthink(&mut self, slot: usize, time: i32);
    /// Borrow the callback catalog.
    fn callbacks(&self) -> &Q3CallbackCatalog;
    /// Mutably borrow the callback catalog.
    fn callbacks_mut(&mut self) -> &mut Q3CallbackCatalog;
    /// Borrow the ranking reports.
    fn rankings(&self) -> &Q3RankingReports;
    /// Restore entity/client counts (`restoreCounts`).
    fn restore_counts(&mut self, num_entities: usize, max_clients: usize);
}

/// Server world surface used by this module (`ServerWorld`).
pub trait ServerWorldOps {
    /// Link an entity (`link`).
    fn link(&mut self, slot: usize);
    /// Unlink an entity (`unlink`).
    fn unlink(&mut self, slot: usize);
    /// Link state (`linkState`).
    fn link_state(&self, slot: usize) -> Option<LinkState>;
    /// Entities in bounds (`areaEntities`).
    fn area_entities(&self, bounds: &Bounds, maximum: usize) -> Vec<usize>;
}

/// Actor spatial queries used by this module (`ActorSpatialQueries`).
pub trait SpatialQueries {
    /// Actors in bounds (`areaActors`).
    fn area_actors(&self, bounds: &Bounds, maximum: usize) -> Vec<ActorId>;
    /// Trace an actor (`traceActor`).
    fn trace_actor(&self, query: &ActorTraceQuery) -> ActorTraceResult;
}

/// Universal game driver: the subsystem surface behind every runtime in this
/// module. It dissolves the donor host/context objects (`MoverServices`,
/// `CombatContext` carriers, `EntityPoolOptions`, `TargetRuntime`, `TriggerHost`,
/// `WeaponHost`, `PersonalPortalHost`, `TeleportContext`, `ItemLifecycleContext`,
/// `LaunchItemContext`, `DropItemContext`) into one object-safe trait so native
/// callbacks can reach every service through a single handle.
pub trait Q3Driver {
    /// Entity pool.
    fn pool(&mut self) -> &mut dyn EntityPool;
    /// Server world.
    fn world(&mut self) -> &mut dyn ServerWorldOps;
    /// Spatial queries.
    fn spatial(&mut self) -> &mut dyn SpatialQueries;
    /// Combat context.
    fn combat(&mut self) -> &mut dyn CombatContext;
    /// Utility scratch rings.
    fn scratch(&mut self) -> &mut GameUtilityScratch;
    /// Mover actor access.
    fn mover_actors(&mut self) -> &mut dyn MoverActorAccess;
    /// Warn (`warn`).
    fn warn(&mut self, message: &str);
    /// Log (`log`).
    fn log(&mut self, message: &str);
    /// Sound index (`soundIndex`).
    fn sound_index(&mut self, path: &str) -> i32;
    /// Model index (`modelIndex`).
    fn model_index(&mut self, name: Option<&str>) -> i32;
    /// Gravity (`gravity()`).
    fn gravity(&self) -> f32;
    /// Game rand (`rand`).
    fn game_rand(&mut self) -> i32;
    /// Game random (`random`).
    fn game_random(&mut self) -> f32;
    /// Game crandom (`crandom`).
    fn game_crandom(&mut self) -> f32;
    /// Remap a shader (`remapShader`).
    fn remap_shader(&mut self, old: &str, new: &str, time_seconds: f32);
    /// Set a configstring (`setConfigstring`).
    fn set_configstring(&mut self, index: i32, value: &str);
    /// Set a cvar (`setCvar`).
    fn set_cvar(&mut self, name: &str, value: &str);
    /// Send a server command (`sendServerCommand`).
    fn send_server_command(&mut self, client: i32, command: &str);
    /// Use targets (`useTargets`).
    fn use_targets(&mut self, slot: usize, activator: Option<Participant>);
    /// Adjust an area portal (`adjustAreaPortalState`).
    fn adjust_area_portal(&mut self, slot: usize, open: bool);
    /// Return a dropped flag (`returnDroppedFlag`).
    fn return_dropped_flag(&mut self, slot: usize);
    /// Return a team flag (`returnFlag`).
    fn return_flag(&mut self, team: Team);
    /// Add score (`addScore`).
    fn add_score(&mut self, player: usize, origin: Vec3, points: i32);
    /// Explode a missile (`explodeMissile`).
    fn explode_missile(&mut self, slot: usize);
    /// Teleport a player (`teleportPlayer`).
    fn teleport_player(&mut self, player: usize, origin: Vec3, angles: Vec3);
    /// Whether map travel overrides item/portal routing (`mapTravel` present).
    fn map_travel_mode(&self) -> bool;
    /// Map-travel teleport (`mapTravel.teleport`).
    fn map_travel_teleport(&mut self, player: usize, origin: Vec3, angles: Vec3);
    /// Map-travel flag drop (`mapTravel.dropCarriedFlag`).
    fn map_travel_drop_flag(&mut self, player: usize);
    /// Touch an item (`touchItem`).
    fn touch_item(&mut self, item: usize, player: usize, contact: &TouchContact);
    /// Drop an item (`dropItem`).
    fn drop_item(&mut self, entity: usize, item: usize, angle: i32) -> usize;
    /// Dropped-flag think (`droppedFlagThink`).
    fn dropped_flag_think(&mut self, slot: usize);
    /// Check a dropped team item (`checkDroppedTeamItem`).
    fn check_dropped_team_item(&mut self, slot: usize);
    /// Whether an actor is live.
    fn actor_live(&self, actor: &ActorId) -> bool;
    /// Observed origin for a shared actor (`DamageParticipant.origin()`).
    fn actor_origin(&self, actor: &ActorId) -> Option<Vec3>;
    /// Participant for an actor (`actors.participant`).
    fn participant(&self, actor: &ActorId) -> Participant;
    /// Whether an actor is a player.
    fn actor_is_player(&self, actor: &ActorId) -> bool;
    /// Native slot for an actor.
    fn native_slot(&self, actor: &ActorId) -> Option<usize>;
    /// Emit an actor event.
    fn actor_event(&mut self, actor: &ActorId, event: EntityEvent, parm: i32);
    /// Item table length.
    fn item_count(&self) -> usize;
    /// Item at an index (`itemAt`).
    fn item_at(&self, index: usize) -> Option<ItemDefinition>;
    /// Find an item by pickup name (`findItem`).
    fn find_item(&self, pickup_name: &str) -> Option<usize>;
    /// Find an item for a powerup (`findItemForPowerup`).
    fn find_item_for_powerup(&self, powerup: i32) -> Option<usize>;
    /// Set a brush model with an immediate link (`setBrushModel`).
    fn set_brush_model(&mut self, slot: usize, model: Option<&str>);
    /// Fire a hitscan bullet (`q3BulletFire`).
    fn bullet_fire(
        &mut self,
        host: &mut dyn BulletHost,
        shooter: &ActorId,
        attack: &mut Q3BulletAttack,
        spread: i32,
        amount: i32,
    );
    /// Run a gauntlet attack (`q3GauntletAttack`).
    fn gauntlet_attack(
        &mut self,
        host: &mut dyn ContactHost,
        shooter: &ActorId,
        attack: &mut Q3BulletAttack,
        quad: bool,
    ) -> bool;
    /// Fire lightning (`q3LightningFire`).
    fn lightning_fire(&mut self, host: &mut dyn ContactHost, shooter: &ActorId, attack: &mut Q3BulletAttack);
    /// Fire the shotgun (`q3ShotgunFire`).
    fn shotgun_fire(&mut self, host: &mut dyn ShotgunHost, shooter: &ActorId, attack: &mut Q3BulletAttack);
    /// Fire the railgun (`q3RailFire`).
    fn rail_fire(&mut self, host: &mut dyn RailHost, shooter: &ActorId, attack: &mut Q3BulletAttack) -> i32;
    /// Compute rail statistics (`q3RailStatistics`).
    fn rail_statistics(&mut self, state: &RailStatistics, hits: i32, time: i32) -> RailStatisticsOutcome;
}

/// Require a live entity slot (`owned` / `requireOwned` checks).
pub fn require_entity(pool: &dyn EntityPool, slot: usize) -> Result<(), Q3GameError> {
    if pool.entity(slot).is_none() {
        return Err(failure(format!(
            "entity {slot} does not belong to its entity pool or was replaced"
        )));
    }
    Ok(())
}

/// Store a fixed trajectory and current collision origin (`setOrigin`).
pub fn set_origin(entity: &mut GameEntity, origin: Vec3) {
    entity.s.pos = Trajectory {
        trajectory_type: TrajectoryType::TrStationary,
        time: 0,
        duration: 0,
        base: origin,
        delta: vec3(0.0, 0.0, 0.0),
    };
    entity.r.current_origin = origin;
}

/// Run a scheduled think (`runThink` + the record `runThink` thunk).
pub fn run_think(driver: &mut dyn Q3Driver, slot: usize, time: i32) -> Result<(), Q3GameError> {
    let nextthink = driver.pool().entity(slot).map(|entity| entity.nextthink).unwrap_or(0);
    if nextthink <= 0 || nextthink > time {
        return Ok(());
    }
    let think = driver.pool().entity(slot).and_then(|entity| entity.think.clone());
    driver.pool().set_nextthink(slot, 0);
    match think {
        Some(think) => {
            think(driver, slot);
            Ok(())
        }
        None => Err(failure("NULL ent->think")),
    }
}

/// Whether an entity rides a support actor (`rides`).
#[must_use]
pub fn rides(entity: &GameEntity, support: &ActorId) -> bool {
    entity.r.ground.as_ref().is_some_and(|ground| ground == support)
}

/// Clear an entity's support with the delayed-continuation marker (`loseGround`).
pub fn lose_ground(entity: &mut GameEntity) {
    entity.r.ground = None;
    entity.s.ground_entity_num = -1;
}

/// Apply jump-pad velocity (`touchJumpPad`).
pub fn touch_jump_pad(state: &mut Q3PlayerState, jump_pad: &EntityState) {
    if state.pm_type != MoveType::PmNormal as i32 || state.powerups.get(Powerup::PwFlight as usize) != 0 {
        return;
    }
    if state.jumppad_ent != jump_pad.number {
        let pitch = angle_normalize180(f64::from(vector_to_angles(jump_pad.origin2).x)).abs();
        state.add_event(EntityEvent::EvJumpPad as i32, i32::from(pitch >= 45.0));
    }
    state.jumppad_ent = jump_pad.number;
    state.jumppad_frame = state.pmove_framecount;
    state.velocity = jump_pad.origin2;
}

/// Shared-actor combat state for accuracy subjects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorCombatState {
    /// Can take damage.
    pub can_take_damage: bool,
    /// Health.
    pub health: i32,
    /// Team number, when teamed.
    pub team: Option<i32>,
}

/// Combat surface used by this module (`CombatContext` + `damage`).
pub trait CombatContext {
    /// Product.
    fn product(&self) -> Product;
    /// Game type number.
    fn game_type(&self) -> i32;
    /// Time.
    fn time(&self) -> i32;
    /// Apply damage (`damage`); normalizes `direction` in place.
    #[allow(clippy::too_many_arguments)]
    fn damage(
        &mut self,
        target: &Participant,
        inflictor: Option<&Participant>,
        attacker: Option<&Participant>,
        direction: Option<&mut Vec3>,
        point: Option<Vec3>,
        amount: i32,
        flags: i32,
        method: i32,
    );
    /// Shared-actor combat state (`authority.read` projection).
    fn actor_combat_state(&self, actor: &ActorId) -> Option<ActorCombatState>;
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::q3::base::game::save_state::*;
    use crate::q3::base::game::spawn::*;

    use crate::q3::base::game::mover::*;
    use crate::q3::base::game::numeric::*;
    use qa_core::identity::ActorId;
    use qa_core::identity::SavedActorId;
    use qa_core::math::normalize3;
    use qa_core::math::vec3;
    use qa_core::math::Bounds;
    use qa_core::math::Vec3;

    use std::collections::HashMap;

    use crate::q3::base::game::entities::ItemDefinition;
    use crate::q3::base::game::hitscan::{Q3BulletAttack, RailStatistics, RailStatisticsOutcome};
    use crate::q3::base::game::memory::GameMemory;
    use crate::q3::base::game::missile::MissileLauncher;
    use crate::q3::base::shared::definitions::{EntityEvent, ItemType, Product, Team, DEFAULT_GRAVITY};
    use crate::q3::base::world::{
        ActorTraceHit, ActorTraceQuery, ActorTraceResult, LinkState, TraceContact, TraceSolidity,
    };
    use qa_core::identity::IdentityOwner;
    use std::rc::Rc;

    pub(crate) struct StubPool {
        pub(crate) product: Product,
        pub(crate) entities: Vec<GameEntity>,
        pub(crate) clients: Vec<GameClient>,
        pub(crate) callbacks: Q3CallbackCatalog,
        pub(crate) rankings: Q3RankingReports,
        pub(crate) num_entities: usize,
        pub(crate) events: Vec<(usize, i32, i32)>,
        pub(crate) scheduled: Vec<(usize, i32)>,
        pub(crate) freed: Vec<usize>,
    }

    impl StubPool {
        pub(crate) fn new(owner: &IdentityOwner, product: Product) -> Self {
            let mut entities = Vec::with_capacity(MAX_GENTITIES);
            for slot in 0..MAX_GENTITIES {
                let mut entity = GameEntity::new(slot, owner.actor(slot as u32, 1)).expect("slot");
                entity.s.number = slot as i32;
                entities.push(entity);
            }
            let mut clients = Vec::with_capacity(MAX_CLIENTS);
            for _ in 0..MAX_CLIENTS {
                clients.push(GameClient::new(product));
            }
            Self {
                product,
                entities,
                clients,
                callbacks: Q3CallbackCatalog::new(),
                rankings: Q3RankingReports::new(),
                num_entities: 0,
                events: Vec::new(),
                scheduled: Vec::new(),
                freed: Vec::new(),
            }
        }

        pub(crate) fn use_slot(&mut self, slot: usize) {
            self.entities[slot].inuse = true;
            self.num_entities = self.num_entities.max(slot + 1);
        }
    }

    impl EntityPool for StubPool {
        fn product(&self) -> Product {
            self.product
        }
        fn num_entities(&self) -> usize {
            self.num_entities
        }
        fn max_clients(&self) -> usize {
            MAX_CLIENTS
        }
        fn entity(&self, slot: usize) -> Option<&GameEntity> {
            self.entities.get(slot)
        }
        fn entity_mut(&mut self, slot: usize) -> Option<&mut GameEntity> {
            self.entities.get_mut(slot)
        }
        fn client(&self, slot: usize) -> Option<&GameClient> {
            self.clients.get(slot)
        }
        fn client_mut(&mut self, slot: usize) -> Option<&mut GameClient> {
            self.clients.get_mut(slot)
        }
        fn spawn_entity(&mut self) -> Result<usize, Q3GameError> {
            for slot in 0..MAX_GENTITIES {
                if !self.entities[slot].inuse {
                    self.entities[slot].inuse = true;
                    self.num_entities = self.num_entities.max(slot + 1);
                    return Ok(slot);
                }
            }
            Err(failure("G_Spawn: no free entities"))
        }
        fn free_entity(&mut self, slot: usize) {
            if let Some(entity) = self.entities.get_mut(slot) {
                entity.inuse = false;
            }
            self.freed.push(slot);
        }
        fn temp_entity(&mut self, origin: Vec3, event: EntityEvent) -> usize {
            let slot = self.spawn_entity().expect("temp slot");
            self.entities[slot].s.origin = origin;
            self.entities[slot].s.event = event as i32;
            slot
        }
        fn add_event(&mut self, slot: usize, event: EntityEvent, parm: i32) {
            self.events.push((slot, event as i32, parm));
        }
        fn set_nextthink(&mut self, slot: usize, time: i32) {
            if let Some(entity) = self.entities.get_mut(slot) {
                entity.nextthink = time;
            }
            self.scheduled.push((slot, time));
        }
        fn callbacks(&self) -> &Q3CallbackCatalog {
            &self.callbacks
        }
        fn callbacks_mut(&mut self) -> &mut Q3CallbackCatalog {
            &mut self.callbacks
        }
        fn rankings(&self) -> &Q3RankingReports {
            &self.rankings
        }
        fn restore_counts(&mut self, num_entities: usize, _max_clients: usize) {
            self.num_entities = num_entities;
        }
    }

    #[derive(Clone)]
    pub(crate) struct DamageCall {
        pub(crate) amount: i32,
        pub(crate) method: i32,
    }

    pub(crate) struct StubCombat {
        pub(crate) product: Product,
        pub(crate) game_type: i32,
        pub(crate) time: i32,
        pub(crate) calls: Vec<DamageCall>,
        pub(crate) states: HashMap<ActorId, ActorCombatState>,
    }

    impl CombatContext for StubCombat {
        fn product(&self) -> Product {
            self.product
        }
        fn game_type(&self) -> i32 {
            self.game_type
        }
        fn time(&self) -> i32 {
            self.time
        }
        fn damage(
            &mut self,
            _target: &Participant,
            _inflictor: Option<&Participant>,
            _attacker: Option<&Participant>,
            direction: Option<&mut Vec3>,
            _point: Option<Vec3>,
            amount: i32,
            _flags: i32,
            method: i32,
        ) {
            if let Some(direction) = direction {
                *direction = normalize3(*direction);
            }
            self.calls.push(DamageCall { amount, method });
        }
        fn actor_combat_state(&self, actor: &ActorId) -> Option<ActorCombatState> {
            self.states.get(actor).cloned()
        }
    }

    pub(crate) struct StubWorld {
        pub(crate) linked: std::collections::HashSet<usize>,
        pub(crate) links: HashMap<usize, LinkState>,
        pub(crate) area: Vec<usize>,
    }

    impl ServerWorldOps for StubWorld {
        fn link(&mut self, slot: usize) {
            self.linked.insert(slot);
        }
        fn unlink(&mut self, slot: usize) {
            self.linked.remove(&slot);
        }
        fn link_state(&self, slot: usize) -> Option<LinkState> {
            self.links.get(&slot).cloned()
        }
        fn area_entities(&self, _bounds: &Bounds, _maximum: usize) -> Vec<usize> {
            self.area.clone()
        }
    }

    pub(crate) struct StubSpatial {
        pub(crate) trace_result: ActorTraceResult,
        pub(crate) area: Vec<ActorId>,
    }

    pub(crate) fn clear_trace(end: Vec3) -> ActorTraceResult {
        ActorTraceResult {
            fraction: 1.0,
            end,
            solidity: TraceSolidity::Clear,
            contact: TraceContact::None,
            contents: 0,
            surface_flags: 0,
            hit: ActorTraceHit::None,
        }
    }

    impl SpatialQueries for StubSpatial {
        fn area_actors(&self, _bounds: &Bounds, _maximum: usize) -> Vec<ActorId> {
            self.area.clone()
        }
        fn trace_actor(&self, _query: &ActorTraceQuery) -> ActorTraceResult {
            self.trace_result.clone()
        }
    }

    pub(crate) struct StubMoverActors {
        pub(crate) natives: HashMap<ActorId, usize>,
        pub(crate) bodies: HashMap<ActorId, SharedMoverBody>,
        pub(crate) writes: Vec<(ActorId, Vec3)>,
        pub(crate) released: Vec<ActorId>,
    }

    impl MoverActorAccess for StubMoverActors {
        fn native_slot(&self, actor: &ActorId) -> Option<usize> {
            self.natives.get(actor).copied()
        }
        fn participant(&self, actor: &ActorId) -> Participant {
            match self.natives.get(actor) {
                Some(slot) => Participant::Entity(*slot),
                None => Participant::SharedActor(actor.clone()),
            }
        }
        fn observe(&self, actor: &ActorId) -> Option<SharedMoverBody> {
            self.bodies.get(actor).cloned()
        }
        fn write(&mut self, actor: &ActorId, origin: Vec3, ground: Option<ActorId>) {
            self.writes.push((actor.clone(), origin));
            if let Some(body) = self.bodies.get_mut(actor) {
                body.state.origin = origin;
                body.state.ground = ground;
            }
        }
        fn link_actor(&mut self, _actor: &ActorId) {}
        fn release(&mut self, actor: &ActorId) {
            self.released.push(actor.clone());
        }
    }

    pub(crate) struct StubDriver {
        pub(crate) pool: StubPool,
        pub(crate) world: StubWorld,
        pub(crate) spatial: StubSpatial,
        pub(crate) combat: StubCombat,
        pub(crate) scratch: GameUtilityScratch,
        pub(crate) actors: StubMoverActors,
        pub(crate) warns: Vec<String>,
        pub(crate) logs: Vec<String>,
        pub(crate) sounds: HashMap<String, i32>,
        pub(crate) models: HashMap<String, i32>,
        pub(crate) gravity: f32,
        pub(crate) random: GameRandom,
        pub(crate) remaps: Vec<(String, String, f32)>,
        pub(crate) configstrings: HashMap<i32, String>,
        pub(crate) cvars: HashMap<String, String>,
        pub(crate) commands: Vec<(i32, String)>,
        pub(crate) use_targets_calls: Vec<(usize, Option<Participant>)>,
        pub(crate) portals: Vec<(usize, bool)>,
        pub(crate) dropped_flags: Vec<usize>,
        pub(crate) returned_flags: Vec<Team>,
        pub(crate) scores: Vec<(usize, i32)>,
        pub(crate) exploded: Vec<usize>,
        pub(crate) teleports: Vec<(usize, Vec3, Vec3)>,
        pub(crate) map_travel: bool,
        pub(crate) touched_items: Vec<(usize, usize)>,
        pub(crate) dropped_items: Vec<(usize, usize, i32)>,
        pub(crate) flag_thinks: Vec<usize>,
        pub(crate) checked_items: Vec<usize>,
        pub(crate) live: HashMap<ActorId, bool>,
        pub(crate) players: HashMap<ActorId, bool>,
        pub(crate) native_map: HashMap<ActorId, usize>,
        pub(crate) origins: HashMap<ActorId, Vec3>,
        pub(crate) actor_events: Vec<(ActorId, i32, i32)>,
        pub(crate) items: Vec<ItemDefinition>,
        pub(crate) brush_models: Vec<(usize, Option<String>)>,
        pub(crate) bullet_calls: Vec<(i32, i32)>,
        pub(crate) gauntlet_result: bool,
        pub(crate) lightning_calls: usize,
        pub(crate) shotgun_calls: usize,
        pub(crate) rail_hits: i32,
        pub(crate) rail_outcome: RailStatisticsOutcome,
    }

    impl StubDriver {
        pub(crate) fn new(owner: &IdentityOwner, product: Product) -> Self {
            Self {
                pool: StubPool::new(owner, product),
                world: StubWorld {
                    linked: std::collections::HashSet::new(),
                    links: HashMap::new(),
                    area: Vec::new(),
                },
                spatial: StubSpatial {
                    trace_result: clear_trace(vec3(0.0, 0.0, 0.0)),
                    area: Vec::new(),
                },
                combat: StubCombat {
                    product,
                    game_type: 0,
                    time: 1000,
                    calls: Vec::new(),
                    states: HashMap::new(),
                },
                scratch: GameUtilityScratch::new(Rc::new(|_| {})),
                actors: StubMoverActors {
                    natives: HashMap::new(),
                    bodies: HashMap::new(),
                    writes: Vec::new(),
                    released: Vec::new(),
                },
                warns: Vec::new(),
                logs: Vec::new(),
                sounds: HashMap::new(),
                models: HashMap::new(),
                gravity: DEFAULT_GRAVITY as f32,
                random: GameRandom::new(0),
                remaps: Vec::new(),
                configstrings: HashMap::new(),
                cvars: HashMap::new(),
                commands: Vec::new(),
                use_targets_calls: Vec::new(),
                portals: Vec::new(),
                dropped_flags: Vec::new(),
                returned_flags: Vec::new(),
                scores: Vec::new(),
                exploded: Vec::new(),
                teleports: Vec::new(),
                map_travel: false,
                touched_items: Vec::new(),
                dropped_items: Vec::new(),
                flag_thinks: Vec::new(),
                checked_items: Vec::new(),
                live: HashMap::new(),
                players: HashMap::new(),
                native_map: HashMap::new(),
                origins: HashMap::new(),
                actor_events: Vec::new(),
                items: Vec::new(),
                brush_models: Vec::new(),
                bullet_calls: Vec::new(),
                gauntlet_result: false,
                lightning_calls: 0,
                shotgun_calls: 0,
                rail_hits: 0,
                rail_outcome: RailStatisticsOutcome {
                    streak: 0,
                    hits: 0,
                    impressive_count: 0,
                    reward_until: 0,
                    awarded: false,
                },
            }
        }
    }

    impl Q3Driver for StubDriver {
        fn pool(&mut self) -> &mut dyn EntityPool {
            &mut self.pool
        }
        fn world(&mut self) -> &mut dyn ServerWorldOps {
            &mut self.world
        }
        fn spatial(&mut self) -> &mut dyn SpatialQueries {
            &mut self.spatial
        }
        fn combat(&mut self) -> &mut dyn CombatContext {
            &mut self.combat
        }
        fn scratch(&mut self) -> &mut GameUtilityScratch {
            &mut self.scratch
        }
        fn mover_actors(&mut self) -> &mut dyn MoverActorAccess {
            &mut self.actors
        }
        fn warn(&mut self, message: &str) {
            self.warns.push(message.to_string());
        }
        fn log(&mut self, message: &str) {
            self.logs.push(message.to_string());
        }
        fn sound_index(&mut self, path: &str) -> i32 {
            let next = self.sounds.len() as i32 + 1;
            *self.sounds.entry(path.to_string()).or_insert(next)
        }
        fn model_index(&mut self, name: Option<&str>) -> i32 {
            let next = self.models.len() as i32 + 1;
            *self.models.entry(name.unwrap_or("").to_string()).or_insert(next)
        }
        fn gravity(&self) -> f32 {
            self.gravity
        }
        fn game_rand(&mut self) -> i32 {
            self.random.rand()
        }
        fn game_random(&mut self) -> f32 {
            self.random.random()
        }
        fn game_crandom(&mut self) -> f32 {
            self.random.crandom()
        }
        fn remap_shader(&mut self, old: &str, new: &str, time_seconds: f32) {
            self.remaps.push((old.to_string(), new.to_string(), time_seconds));
        }
        fn set_configstring(&mut self, index: i32, value: &str) {
            self.configstrings.insert(index, value.to_string());
        }
        fn set_cvar(&mut self, name: &str, value: &str) {
            self.cvars.insert(name.to_string(), value.to_string());
        }
        fn send_server_command(&mut self, client: i32, command: &str) {
            self.commands.push((client, command.to_string()));
        }
        fn use_targets(&mut self, slot: usize, activator: Option<Participant>) {
            self.use_targets_calls.push((slot, activator));
        }
        fn adjust_area_portal(&mut self, slot: usize, open: bool) {
            self.portals.push((slot, open));
        }
        fn return_dropped_flag(&mut self, slot: usize) {
            self.dropped_flags.push(slot);
        }
        fn return_flag(&mut self, team: Team) {
            self.returned_flags.push(team);
        }
        fn add_score(&mut self, player: usize, _origin: Vec3, points: i32) {
            self.scores.push((player, points));
        }
        fn explode_missile(&mut self, slot: usize) {
            self.exploded.push(slot);
        }
        fn teleport_player(&mut self, player: usize, origin: Vec3, angles: Vec3) {
            self.teleports.push((player, origin, angles));
        }
        fn map_travel_mode(&self) -> bool {
            self.map_travel
        }
        fn map_travel_teleport(&mut self, player: usize, origin: Vec3, angles: Vec3) {
            self.teleports.push((player, origin, angles));
        }
        fn map_travel_drop_flag(&mut self, player: usize) {
            self.dropped_flags.push(player);
        }
        fn touch_item(&mut self, item: usize, player: usize, _contact: &TouchContact) {
            self.touched_items.push((item, player));
        }
        fn drop_item(&mut self, entity: usize, item: usize, angle: i32) -> usize {
            self.dropped_items.push((entity, item, angle));
            self.pool.spawn_entity().expect("drop slot")
        }
        fn dropped_flag_think(&mut self, slot: usize) {
            self.flag_thinks.push(slot);
        }
        fn check_dropped_team_item(&mut self, slot: usize) {
            self.checked_items.push(slot);
        }
        fn actor_live(&self, actor: &ActorId) -> bool {
            self.live.get(actor).copied().unwrap_or(false)
        }
        fn actor_origin(&self, actor: &ActorId) -> Option<Vec3> {
            self.origins.get(actor).copied()
        }
        fn participant(&self, actor: &ActorId) -> Participant {
            match self.native_map.get(actor) {
                Some(slot) => Participant::Entity(*slot),
                None => Participant::SharedActor(actor.clone()),
            }
        }
        fn actor_is_player(&self, actor: &ActorId) -> bool {
            self.players.get(actor).copied().unwrap_or(false)
        }
        fn native_slot(&self, actor: &ActorId) -> Option<usize> {
            self.native_map.get(actor).copied()
        }
        fn actor_event(&mut self, actor: &ActorId, event: EntityEvent, parm: i32) {
            self.actor_events.push((actor.clone(), event as i32, parm));
        }
        fn item_count(&self) -> usize {
            self.items.len()
        }
        fn item_at(&self, index: usize) -> Option<ItemDefinition> {
            self.items.get(index).cloned()
        }
        fn find_item(&self, pickup_name: &str) -> Option<usize> {
            self.items
                .iter()
                .position(|item| item.pickup_name.as_deref() == Some(pickup_name))
        }
        fn find_item_for_powerup(&self, powerup: i32) -> Option<usize> {
            self.items
                .iter()
                .position(|item| item.item_type == ItemType::ItPowerup && item.tag == powerup)
        }
        fn set_brush_model(&mut self, slot: usize, model: Option<&str>) {
            self.brush_models.push((slot, model.map(str::to_string)));
        }
        fn bullet_fire(
            &mut self,
            _host: &mut dyn BulletHost,
            _shooter: &ActorId,
            _attack: &mut Q3BulletAttack,
            spread: i32,
            amount: i32,
        ) {
            self.bullet_calls.push((spread, amount));
        }
        fn gauntlet_attack(
            &mut self,
            _host: &mut dyn ContactHost,
            _shooter: &ActorId,
            _attack: &mut Q3BulletAttack,
            _quad: bool,
        ) -> bool {
            self.gauntlet_result
        }
        fn lightning_fire(&mut self, _host: &mut dyn ContactHost, _shooter: &ActorId, _attack: &mut Q3BulletAttack) {
            self.lightning_calls += 1;
        }
        fn shotgun_fire(&mut self, _host: &mut dyn ShotgunHost, _shooter: &ActorId, _attack: &mut Q3BulletAttack) {
            self.shotgun_calls += 1;
        }
        fn rail_fire(&mut self, _host: &mut dyn RailHost, _shooter: &ActorId, _attack: &mut Q3BulletAttack) -> i32 {
            self.rail_hits
        }
        fn rail_statistics(&mut self, _state: &RailStatistics, _hits: i32, _time: i32) -> RailStatisticsOutcome {
            self.rail_outcome
        }
    }

    pub(crate) struct StubLauncher {
        pub(crate) fires: Vec<String>,
        pub(crate) damage: i32,
        pub(crate) splash: i32,
    }

    impl MissileLauncher for StubLauncher {
        fn fire_grenade(
            &mut self,
            driver: &mut dyn Q3Driver,
            _entity: usize,
            _muzzle: Vec3,
            _direction: Vec3,
        ) -> usize {
            self.fires.push("grenade".to_string());
            let slot = driver.pool().spawn_entity().expect("missile");
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.damage = self.damage;
                entity.splash_damage = self.splash;
            }
            slot
        }
        fn fire_rocket(&mut self, driver: &mut dyn Q3Driver, _entity: usize, _muzzle: Vec3, _direction: Vec3) -> usize {
            self.fires.push("rocket".to_string());
            let slot = driver.pool().spawn_entity().expect("missile");
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.damage = self.damage;
                entity.splash_damage = self.splash;
            }
            slot
        }
        fn fire_plasma(&mut self, driver: &mut dyn Q3Driver, _entity: usize, _muzzle: Vec3, _direction: Vec3) -> usize {
            self.fires.push("plasma".to_string());
            let slot = driver.pool().spawn_entity().expect("missile");
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.damage = self.damage;
                entity.splash_damage = self.splash;
            }
            slot
        }
        fn fire_bfg(&mut self, driver: &mut dyn Q3Driver, _entity: usize, _muzzle: Vec3, _direction: Vec3) -> usize {
            self.fires.push("bfg".to_string());
            let slot = driver.pool().spawn_entity().expect("missile");
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.damage = self.damage;
                entity.splash_damage = self.splash;
            }
            slot
        }
        fn fire_grapple(
            &mut self,
            driver: &mut dyn Q3Driver,
            _entity: usize,
            _muzzle: Vec3,
            _direction: Vec3,
        ) -> usize {
            self.fires.push("grapple".to_string());
            driver.pool().spawn_entity().expect("missile")
        }
        fn fire_nail(
            &mut self,
            driver: &mut dyn Q3Driver,
            _entity: usize,
            _muzzle: Vec3,
            _forward: Vec3,
            _right: Vec3,
            _up: Vec3,
        ) -> usize {
            self.fires.push("nail".to_string());
            let slot = driver.pool().spawn_entity().expect("missile");
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.damage = self.damage;
                entity.splash_damage = self.splash;
            }
            slot
        }
        fn fire_prox(&mut self, driver: &mut dyn Q3Driver, _entity: usize, _muzzle: Vec3, _direction: Vec3) -> usize {
            self.fires.push("prox".to_string());
            let slot = driver.pool().spawn_entity().expect("missile");
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.damage = self.damage;
                entity.splash_damage = self.splash;
            }
            slot
        }
    }

    pub(crate) struct StubRecords {
        pub(crate) product: Product,
        pub(crate) items: usize,
        pub(crate) ownership: Vec<OwnershipEntry>,
        pub(crate) backing: HashMap<usize, ClientBacking>,
        pub(crate) callbacks_restored: bool,
    }

    impl Q3EntityRecords for StubRecords {
        fn product(&self) -> Product {
            self.product
        }
        fn item_count(&self) -> usize {
            self.items
        }
        fn capture_ownership(&self) -> Vec<OwnershipEntry> {
            self.ownership.clone()
        }
        fn restore_ownership(&mut self, entries: Vec<OwnershipEntry>) {
            self.ownership = entries;
        }
        fn capture_client_backing(&self, slot: usize) -> ClientBacking {
            self.backing.get(&slot).cloned().unwrap_or(ClientBacking {
                source_stats: Vec::new(),
                special_ammo: Vec::new(),
            })
        }
        fn restore_client_backing(&mut self, slot: usize, backing: &ClientBacking) {
            self.backing.insert(slot, backing.clone());
        }
        fn damage_inflictor(&self, actor: &ActorId) -> Participant {
            Participant::SharedActor(actor.clone())
        }
        fn restore_callbacks(&mut self) {
            self.callbacks_restored = true;
        }
    }

    pub(crate) struct StubRegistry {
        pub(crate) map: HashMap<SavedActorId, ActorId>,
    }

    impl Q3ActorRegistry for StubRegistry {
        fn resolve_saved(&self, saved: &SavedActorId) -> Option<ActorId> {
            self.map.get(saved).cloned()
        }
        fn reference_saved(&self, saved: &SavedActorId) -> ActorId {
            self.map.get(saved).cloned().expect("known actor")
        }
    }

    pub(crate) struct StubSpawnServices {
        pub(crate) memory: GameMemory,
        pub(crate) product: Product,
        pub(crate) game_type: i32,
        pub(crate) handlers: SpawnHandlerTable,
        pub(crate) spawned_items: Vec<(usize, usize)>,
        pub(crate) warns: Vec<String>,
    }

    impl SpawnServices for StubSpawnServices {
        fn memory(&mut self) -> &mut GameMemory {
            &mut self.memory
        }
        fn product(&self) -> Product {
            self.product
        }
        fn game_type(&self) -> i32 {
            self.game_type
        }
        fn handlers(&self) -> &SpawnHandlerTable {
            &self.handlers
        }
        fn spawn_item(
            &mut self,
            _driver: &mut dyn Q3Driver,
            slot: usize,
            item: usize,
            _variables: &SpawnVariables,
        ) -> Result<(), Q3GameError> {
            self.spawned_items.push((slot, item));
            Ok(())
        }
        fn warn(&mut self, message: &str) {
            self.warns.push(message.to_string());
        }
    }

    pub(crate) fn test_owner() -> IdentityOwner {
        IdentityOwner::create("test").expect("owner")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::q3::base::game::state::test_support::*;

    use crate::q3::base::shared::definitions::Product;
    use std::rc::Rc;

    #[test]
    fn state_defaults_and_callbacks() {
        let owner = test_owner();
        let actor = owner.actor(3, 1);
        let entity = GameEntity::new(3, actor.clone()).unwrap();
        assert_eq!(entity.mover_state, MoverState::Pos1 as i32);
        assert!(GameEntity::new(5000, actor).is_err());
        let client = GameClient::new(Product::Baseq3);
        assert_eq!(client.ammo_times.len(), 11);
        assert_eq!(GameClient::new(Product::Missionpack).ammo_times.len(), 14);
        let mut catalog = Q3CallbackCatalog::new();
        let think: EntityThink = Rc::new(|_, _| {});
        catalog.think.register("id.a", Rc::clone(&think)).unwrap();
        assert!(catalog.think.register("id.b", Rc::clone(&think)).is_err());
        assert!(catalog.think.register("", Rc::clone(&think)).is_err());
        assert_eq!(catalog.think.capture(Some(&think)).unwrap(), Some("id.a".to_string()));
        assert!(catalog.think.resolve(Some("missing")).is_err());
        assert!(catalog.think.resolve(None).unwrap().is_none());
        let mut ps = Q3PlayerState::new(Product::Baseq3);
        ps.stats.set(0, 77);
        assert_eq!(ps.health(), 77);
        ps.add_event(5, 6);
        assert_eq!(ps.event_sequence, 1);
        assert_eq!(ps.events.get(0), 5);
    }
}
