//! Quake III base/game: state.
//!
//! Donor provenance: `src/content/q3/base/game/state.ts`.

use qa_core::identity::ActorId;
use qa_core::math::vec3;
use qa_core::math::Vec3;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::mirrors_game_state::*;

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
            session_team: Q3Team::Free as i32,
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
    pub fn new(product: Q3Product) -> Self {
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
            ammo_times: Q3PlayerSlots::new(weapon_count(product)),
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
    pub s: Q3EntityState,
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
            s: Q3EntityState::default(),
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
pub fn create_game_client(product: Q3Product) -> GameClient {
    GameClient::new(product)
}

/// Create a game entity (`createGameEntity`).
pub fn create_game_entity(slot: usize, actor: ActorId) -> Result<GameEntity, Q3GameError> {
    GameEntity::new(slot, actor)
}
