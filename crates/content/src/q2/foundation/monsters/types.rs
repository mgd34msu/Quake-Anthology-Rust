//! Monster frame data types (`src/content/q2/foundation/monsters/types.ts`).
//!
//! Species animation tables share this vocabulary; the runners live in
//! `moves`, `frames`, `ai` and `perception`.

use std::collections::HashMap;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::monsters::MonsterMission;
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{Q2Entity, Q2GameServices};
use crate::q2::foundation::weapons::types::Q2GrenadeAdjustment;
use crate::q2::support::contracts::{
    DeathReaction, PainReaction, PowerArmorCells, TraceResult,
};

/// Monster frame AI selector (`MonsterFrame["ai"]`).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum MonsterAi {
    /// Standing AI.
    Stand,
    /// Walking AI.
    Walk,
    /// Running AI.
    Run,
    /// Charging AI.
    Charge,
    /// Scripted-move AI.
    Move,
    /// Soldier sidestep AI.
    SoldierMove,
    /// Turning AI.
    Turn,
    /// No AI.
    #[default]
    None,
    /// Named source AI.
    Source(String),
}

impl MonsterAi {
    /// Donor table spelling (checkpoints store `soldier_move` verbatim).
    pub fn as_str(&self) -> &str {
        match self {
            MonsterAi::Stand => "stand",
            MonsterAi::Walk => "walk",
            MonsterAi::Run => "run",
            MonsterAi::Charge => "charge",
            MonsterAi::Move => "move",
            MonsterAi::SoldierMove => "soldier_move",
            MonsterAi::Turn => "turn",
            MonsterAi::None => "none",
            MonsterAi::Source(_) => "source",
        }
    }
}

/// Monster frame action (`MonsterFrame["actions"]`).
#[derive(Debug, Clone, PartialEq)]
pub enum MonsterAction {
    /// Named callback.
    Name(String),
    /// Explicit next frame (`"next"` keeps sequencing relative).
    NextFrame(NextFrame),
}

/// Explicit next-frame target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NextFrame {
    /// Absolute frame.
    Frame(i32),
    /// Next sequential frame.
    Next,
}

impl MonsterAction {
    /// Named frame callback.
    pub fn name(name: &str) -> Self {
        MonsterAction::Name(name.to_string())
    }
}

/// One animation frame (`MonsterFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct MonsterFrame {
    /// Frame AI.
    pub ai: MonsterAi,
    /// Frame distance.
    pub distance: f64,
    /// Frame actions.
    pub actions: Vec<MonsterAction>,
    /// Lerp frame.
    pub lerp_frame: i32,
}

impl Default for MonsterFrame {
    fn default() -> Self {
        Self {
            ai: MonsterAi::None,
            distance: 0.0,
            actions: Vec::new(),
            lerp_frame: -1,
        }
    }
}

/// One named animation (`MonsterMove`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MonsterMove {
    /// Move name.
    pub name: String,
    /// First frame.
    pub first_frame: i32,
    /// Last frame.
    pub last_frame: i32,
    /// End callback name.
    pub end: Option<String>,
    /// Sidestep scale.
    pub sidestep_scale: f64,
    /// Frames.
    pub frames: Vec<MonsterFrame>,
}

/// Build one animation frame (species table shorthand).
pub fn monster_frame(
    ai: MonsterAi,
    distance: f64,
    actions: Vec<MonsterAction>,
    lerp_frame: i32,
) -> MonsterFrame {
    MonsterFrame {
        ai,
        distance,
        actions,
        lerp_frame,
    }
}

/// Build one named animation (species table shorthand).
///
/// All shipped source tables keep `sidestepScale` zero.
pub fn monster_move(
    name: &str,
    first_frame: i32,
    last_frame: i32,
    end: Option<&str>,
    frames: Vec<MonsterFrame>,
) -> MonsterMove {
    MonsterMove {
        name: name.to_string(),
        first_frame,
        last_frame,
        end: end.map(str::to_string),
        sidestep_scale: 0.0,
        frames,
    }
}

/// Monster weapon kind (`MonsterState["weapon"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MonsterWeapon {
    /// Blaster.
    #[default]
    Blaster,
    /// Shotgun.
    Shotgun,
    /// Machine gun.
    Machinegun,
}

impl MonsterWeapon {
    /// Weapon from a soldier classname (`admit`).
    pub fn from_classname(classname: &str) -> Self {
        if classname == "monster_soldier_light" {
            MonsterWeapon::Blaster
        } else if classname == "monster_soldier" {
            MonsterWeapon::Shotgun
        } else {
            MonsterWeapon::Machinegun
        }
    }
}

/// Monster locomotion (`MonsterState["locomotion"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MonsterLocomotion {
    /// Walking.
    #[default]
    Walk,
    /// Flying.
    Fly,
    /// Swimming.
    Swim,
    /// Stationary.
    Stationary,
}

/// Monster spawner (`MonsterState["spawnedBy"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MonsterSpawner {
    /// Naturally spawned.
    #[default]
    None,
    /// Spawned by a carrier.
    Carrier,
    /// Spawned by a medic.
    Medic,
    /// Spawned by a widow.
    Widow,
}

/// Monster power armor type (`MonsterState["initialPowerArmorType"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MonsterPowerArmor {
    /// None.
    #[default]
    None,
    /// Screen.
    Screen,
    /// Shield.
    Shield,
}

/// Monster attack state (`MonsterState["attackState"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MonsterAttackState {
    /// Straight attack.
    #[default]
    Straight,
    /// Sliding attack.
    Sliding,
    /// Melee attack.
    Melee,
    /// Missile attack.
    Missile,
    /// Blind-fire attack.
    Blind,
}

/// Flyer follow-up move (`nextMoves` value).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MonsterFlyerNext {
    /// Start melee.
    Melee,
    /// Ranged attack.
    Attack,
    /// Run.
    Run,
}

/// Monster sound target (`MonsterState["soundTarget"]`).
#[derive(Debug, Clone, PartialEq)]
pub struct MonsterSoundTarget {
    /// Sound actor.
    pub actor: ActorId,
    /// Sound owner.
    pub owner: ActorId,
    /// Sound origin.
    pub origin: Vec3,
    /// Sound time.
    pub time: f64,
}

/// Monster fly pathing (`Q2AlternateFlyState["pathing"]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonsterPathing {
    /// First move point.
    pub first_move_point: Vec3,
    /// Second move point.
    pub second_move_point: Vec3,
    /// Whether traversal is pending.
    pub traversal_pending: bool,
}

/// Monster start mode (`startMode` result).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StartMode {
    /// Automatic start.
    Automatic,
    /// Manual start.
    Manual,
}

/// Source combat rules (`sourceCombatRules` result).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SourceCombatMode {
    /// Base rules.
    #[default]
    Base,
    /// Rogue rules.
    Rogue,
}

/// Source move outcome (`beforeSourceMove` result).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SourceMoveOutcome {
    /// The move was handled.
    Handled,
    /// Run the move with a displacement.
    Move {
        /// Displacement.
        displacement: Vec3,
    },
}

/// Platform phase (`platformState` result).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlatformPhase {
    /// At the top.
    Top,
    /// At the bottom.
    Bottom,
    /// Moving up.
    Up,
    /// Moving down.
    Down,
}

/// Dead-think schedule target (`schedule` callback).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeadThink {
    /// Monster dead think.
    MonsterDeadThink,
    /// Flies on.
    FliesOn,
    /// Flies off.
    FliesOff,
}

/// Monster callback (`MonsterHandler` function form).
pub type MonsterCallback = fn(&mut MonsterContext);
/// Monster pain callback.
pub type MonsterPain = fn(&mut MonsterContext, &PainReaction);
/// Monster die callback.
pub type MonsterDie = fn(&mut MonsterContext, &DeathReaction);
/// Named source AI callback.
pub type MonsterAiFn = fn(&mut MonsterContext, f64);
/// Start-mode callback.
pub type MonsterStartMode = fn(&mut MonsterContext) -> StartMode;
/// Duck callback.
pub type MonsterDuck = fn(&mut MonsterContext, f64) -> bool;
/// Sidestep callback.
pub type MonsterSidestep = fn(&mut MonsterContext) -> bool;
/// Dodge callback.
pub type MonsterDodge = fn(&mut MonsterContext, &ActorId, f64, Option<&TraceResult>, bool);
/// Blocked callback.
pub type MonsterBlocked = fn(&mut MonsterContext, f64) -> bool;
/// Check-attack callback.
pub type MonsterCheckAttack = fn(&mut MonsterContext) -> bool;

/// Monster handler (`MonsterHandler`).
///
/// The donor's `move()` and `sound()` factories return closures; those
/// become data here so saves stay structural.
#[derive(Debug, Clone)]
pub enum MonsterHandler {
    /// Plain callback.
    Callback(MonsterCallback),
    /// Set a move (`move(name)`).
    SetMove(String),
    /// Play a sound (`sound(path, channel, attenuation)`).
    PlaySound {
        /// Sound path.
        path: String,
        /// Channel.
        channel: i32,
        /// Attenuation.
        attenuation: f64,
    },
}

impl MonsterHandler {
    /// Dispatch a handler.
    pub fn dispatch(&self, context: &mut MonsterContext) {
        match self {
            MonsterHandler::Callback(callback) => callback(context),
            MonsterHandler::SetMove(name) => context.set_move(name, false),
            MonsterHandler::PlaySound { path, channel, attenuation } => {
                let actor = context.actor().clone();
                context.game.sound(&actor, path, *channel, 1.0, *attenuation);
            }
        }
    }
}

/// Monster definition (`Q2MonsterDefinition`).
#[derive(Debug, Clone)]
pub struct Q2MonsterDefinition {
    /// Classname.
    pub classname: String,
    /// Monster kind.
    pub kind: String,
    /// Model path.
    pub model: String,
    /// Health.
    pub health: f64,
    /// Gib health.
    pub gib_health: f64,
    /// Mass.
    pub mass: f64,
    /// Collision bounds.
    pub bounds: Bounds,
    /// Scale.
    pub scale: f64,
    /// View height.
    pub view_height: Option<i32>,
    /// Yaw speed.
    pub yaw_speed: Option<f64>,
    /// Locomotion.
    pub locomotion: Option<MonsterLocomotion>,
    /// Initial move.
    pub initial_move: String,
    /// Moves.
    pub moves: Vec<MonsterMove>,
    /// Callbacks by name.
    pub callbacks: HashMap<String, MonsterHandler>,
    /// Stand handler.
    pub stand: MonsterHandler,
    /// Walk handler.
    pub walk: MonsterHandler,
    /// Run handler.
    pub run: MonsterHandler,
    /// Attack handler.
    pub attack: MonsterHandler,
    /// Sight handler.
    pub sight: Option<MonsterHandler>,
    /// Idle handler.
    pub idle: Option<MonsterHandler>,
    /// Search handler.
    pub search: Option<MonsterHandler>,
    /// Melee handler.
    pub melee: Option<MonsterHandler>,
    /// Whether the monster has a ranged attack.
    pub has_ranged_attack: bool,
    /// Whether the monster blind-fires.
    pub blind_fire: bool,
    /// Pain handler.
    pub pain: Option<MonsterPain>,
    /// Die handler.
    pub die: MonsterDie,
    /// Named source AI handlers.
    pub ai: HashMap<String, MonsterAiFn>,
    /// Source callbacks.
    pub source_callbacks: Option<Q2CallbackDefinitions>,
    /// Initialize handler.
    pub initialize: Option<MonsterHandler>,
    /// After-spawn handler.
    pub after_spawn: Option<MonsterHandler>,
    /// Start-mode handler.
    pub start_mode: Option<MonsterStartMode>,
    /// Restore handler.
    pub restore: Option<MonsterHandler>,
    /// Duck handler.
    pub duck: Option<MonsterDuck>,
    /// Sidestep handler.
    pub sidestep: Option<MonsterSidestep>,
    /// Dodge handler.
    pub dodge: Option<MonsterDodge>,
    /// Blocked handler.
    pub blocked: Option<MonsterBlocked>,
    /// Check-attack handler.
    pub check_attack: Option<MonsterCheckAttack>,
}

impl Q2MonsterDefinition {
    /// Species definition with donor defaults for every optional slot.
    ///
    /// Species set `sight`, `idle`, `search`, `melee`, `pain`, `ai` and
    /// the rest explicitly after construction.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        classname: &str,
        kind: &str,
        model: &str,
        health: f64,
        gib_health: f64,
        mass: f64,
        bounds: Bounds,
        scale: f64,
        initial_move: &str,
        moves: Vec<MonsterMove>,
        stand: MonsterHandler,
        walk: MonsterHandler,
        run: MonsterHandler,
        attack: MonsterHandler,
        die: MonsterDie,
    ) -> Self {
        Self {
            classname: classname.to_string(),
            kind: kind.to_string(),
            model: model.to_string(),
            health,
            gib_health,
            mass,
            bounds,
            scale,
            view_height: None,
            yaw_speed: None,
            locomotion: None,
            initial_move: initial_move.to_string(),
            moves,
            callbacks: HashMap::new(),
            stand,
            walk,
            run,
            attack,
            sight: None,
            idle: None,
            search: None,
            melee: None,
            has_ranged_attack: false,
            blind_fire: false,
            pain: None,
            die,
            ai: HashMap::new(),
            source_callbacks: None,
            initialize: None,
            after_spawn: None,
            start_mode: None,
            restore: None,
            duck: None,
            sidestep: None,
            dodge: None,
            blocked: None,
            check_attack: None,
        }
    }
}

/// Monster state (`MonsterState`, alternate-fly fields flattened).
#[derive(Debug, Clone)]
pub struct MonsterState {
    /// Monster kind.
    pub kind: String,
    /// Monster weapon.
    pub weapon: MonsterWeapon,
    /// Locomotion.
    pub locomotion: MonsterLocomotion,
    /// Whether the monster has a melee attack.
    pub has_melee: bool,
    /// Whether the monster has a ranged attack.
    pub has_ranged_attack: bool,
    /// Whether the monster has an idle handler.
    pub has_idle: bool,
    /// Whether the monster has a search handler.
    pub has_search: bool,
    /// Whether the monster blind-fires.
    pub blind_fire: bool,
    /// Whether a good guy.
    pub good_guy: bool,
    /// Target anger.
    pub target_anger: bool,
    /// Ignore shots.
    pub ignore_shots: bool,
    /// Do not count.
    pub do_not_count: bool,
    /// Spawner.
    pub spawned_by: MonsterSpawner,
    /// Commander.
    pub commander: Option<ActorId>,
    /// Monster slots.
    pub monster_slots: i32,
    /// Monster used.
    pub monster_used: i32,
    /// Brutal (rerelease nightmare+).
    pub brutal: bool,
    /// Medic.
    pub medic: bool,
    /// Resurrecting.
    pub resurrecting: bool,
    /// Current move.
    pub current_move: MonsterMove,
    /// Next move.
    pub next_move: Option<MonsterMove>,
    /// Next frame.
    pub next_frame: i32,
    /// Next move time.
    pub next_move_time: f64,
    /// Scale.
    pub scale: f64,
    /// Gib health.
    pub gib_health: f64,
    /// Initial power armor type.
    pub initial_power_armor: MonsterPowerArmor,
    /// Maximum power armor power.
    pub max_power_armor_power: f64,
    /// Base health.
    pub base_health: f64,
    /// Health scaling.
    pub health_scaling: f64,
    /// Whether the monster can take damage.
    pub can_take_damage: bool,
    /// Whether dead.
    pub dead: bool,
    /// Whether a corpse.
    pub corpse: bool,
    /// Whether gibbed.
    pub gibbed: bool,
    /// Stand ground.
    pub stand_ground: bool,
    /// Temporary stand ground.
    pub temporary_stand_ground: bool,
    /// Hold frame.
    pub hold_frame: bool,
    /// Ducked.
    pub ducked: bool,
    /// Dodging.
    pub dodging: bool,
    /// Charging.
    pub charging: bool,
    /// Manual steering.
    pub manual_steering: bool,
    /// Combat point.
    pub combat_point: bool,
    /// Attack state.
    pub attack_state: MonsterAttackState,
    /// Lefty.
    pub lefty: bool,
    /// Ideal yaw.
    pub ideal_yaw: f64,
    /// Yaw speed.
    pub yaw_speed: f64,
    /// Pause time.
    pub pause_time: f64,
    /// Idle time.
    pub idle_time: f64,
    /// Pain time.
    pub pain_time: f64,
    /// Fire wait.
    pub fire_wait: f64,
    /// Duck wait.
    pub duck_wait: f64,
    /// Next duck time.
    pub next_duck_time: f64,
    /// Dodge time.
    pub dodge_time: f64,
    /// Attack finished.
    pub attack_finished: f64,
    /// Check attack time.
    pub check_attack_time: f64,
    /// Strafe time.
    pub strafe_time: f64,
    /// Had visibility.
    pub had_visibility: bool,
    /// Close sight tripped.
    pub close_sight_tripped: bool,
    /// Melee time.
    pub melee_time: f64,
    /// Search time.
    pub search_time: f64,
    /// Trail time.
    pub trail_time: f64,
    /// Show hostile.
    pub show_hostile: f64,
    /// Last sighting.
    pub last_sighting: Vec3,
    /// Saved goal.
    pub saved_goal: Option<Vec3>,
    /// Lost sight.
    pub lost_sight: bool,
    /// Pursue next.
    pub pursue_next: bool,
    /// Pursue temporary.
    pub pursue_temporary: bool,
    /// Pursuit last seen.
    pub pursuit_last_seen: bool,
    /// Blind fire target.
    pub blind_fire_target: Vec3,
    /// Blind fire delay.
    pub blind_fire_delay: f64,
    /// Sound target.
    pub sound_target: Option<MonsterSoundTarget>,
    /// Old enemy.
    pub old_enemy: Option<ActorId>,
    /// Move target.
    pub move_target: Option<ActorId>,
    /// Combat target.
    pub combat_target: String,
    /// Cocked.
    pub cocked: bool,
    /// Force refire.
    pub force_refire: bool,
    /// Normal height.
    pub normal_height: f64,
    /// Water level.
    pub water_level: u8,
    /// Water type.
    pub water_type: i32,
    /// Last link count.
    pub last_link_count: i32,
    /// Air finished.
    pub air_finished: f64,
    /// Environmental damage time.
    pub environmental_damage_time: f64,
    /// Jump time.
    pub jump_time: f64,
    /// Flies time.
    pub flies_time: Option<f64>,
    /// Alternate fly behavior (`Q2AlternateFlyState`, flattened).
    pub alternate_fly: bool,
    /// Fly minimum distance.
    pub fly_min_distance: f64,
    /// Fly maximum distance.
    pub fly_max_distance: f64,
    /// Fly acceleration.
    pub fly_acceleration: f64,
    /// Fly speed.
    pub fly_speed: f64,
    /// Fly ideal position.
    pub fly_ideal_position: Vec3,
    /// Fly position time.
    pub fly_position_time: f64,
    /// Fly buzzard.
    pub fly_buzzard: bool,
    /// Fly above.
    pub fly_above: bool,
    /// Fly pinned.
    pub fly_pinned: bool,
    /// Fly thrusters.
    pub fly_thrusters: bool,
    /// Fly recovery time.
    pub fly_recovery_time: f64,
    /// Fly recovery direction.
    pub fly_recovery_direction: Vec3,
    /// Hint path.
    pub hint_path: bool,
    /// Fly pathing.
    pub pathing: Option<MonsterPathing>,
}

impl Default for MonsterState {
    fn default() -> Self {
        Self {
            kind: String::new(),
            weapon: MonsterWeapon::default(),
            locomotion: MonsterLocomotion::default(),
            has_melee: false,
            has_ranged_attack: false,
            has_idle: false,
            has_search: false,
            blind_fire: false,
            good_guy: false,
            target_anger: false,
            ignore_shots: false,
            do_not_count: false,
            spawned_by: MonsterSpawner::default(),
            commander: None,
            monster_slots: 0,
            monster_used: 0,
            brutal: false,
            medic: false,
            resurrecting: false,
            current_move: MonsterMove::default(),
            next_move: None,
            next_frame: 0,
            next_move_time: 0.0,
            scale: 1.0,
            gib_health: 0.0,
            initial_power_armor: MonsterPowerArmor::default(),
            max_power_armor_power: 0.0,
            base_health: 0.0,
            health_scaling: 1.0,
            can_take_damage: false,
            dead: false,
            corpse: false,
            gibbed: false,
            stand_ground: false,
            temporary_stand_ground: false,
            hold_frame: false,
            ducked: false,
            dodging: false,
            charging: false,
            manual_steering: false,
            combat_point: false,
            attack_state: MonsterAttackState::default(),
            lefty: false,
            ideal_yaw: 0.0,
            yaw_speed: 0.0,
            pause_time: 0.0,
            idle_time: 0.0,
            pain_time: 0.0,
            fire_wait: 0.0,
            duck_wait: 0.0,
            next_duck_time: 0.0,
            dodge_time: 0.0,
            attack_finished: 0.0,
            check_attack_time: 0.0,
            strafe_time: 0.0,
            had_visibility: false,
            close_sight_tripped: false,
            melee_time: 0.0,
            search_time: 0.0,
            trail_time: 0.0,
            show_hostile: 0.0,
            last_sighting: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            saved_goal: None,
            lost_sight: false,
            pursue_next: false,
            pursue_temporary: false,
            pursuit_last_seen: false,
            blind_fire_target: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            blind_fire_delay: 0.0,
            sound_target: None,
            old_enemy: None,
            move_target: None,
            combat_target: String::new(),
            cocked: false,
            force_refire: false,
            normal_height: 0.0,
            water_level: 0,
            water_type: 0,
            last_link_count: 0,
            air_finished: 0.0,
            environmental_damage_time: 0.0,
            jump_time: 0.0,
            flies_time: None,
            alternate_fly: false,
            fly_min_distance: 0.0,
            fly_max_distance: 0.0,
            fly_acceleration: 0.0,
            fly_speed: 0.0,
            fly_ideal_position: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            fly_position_time: 0.0,
            fly_buzzard: false,
            fly_above: false,
            fly_pinned: false,
            fly_thrusters: false,
            fly_recovery_time: 0.0,
            fly_recovery_direction: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            hint_path: false,
            pathing: None,
        }
    }
}

/// Monster ballistics bundle (`MonsterWeapons`).
///
/// Eight bound ballistics functions; `Copy` so every transient context
/// carries the bundle without borrowing the runtime.
#[derive(Clone, Copy)]
pub struct MonsterWeapons {
    /// Fire a bullet.
    pub fire_bullet: fn(ActorId, &mut Q2GameServices, Vec3, Vec3, f64, f64, f64, f64, i32),
    /// Fire shotgun pellets.
    pub fire_shotgun:
        fn(ActorId, &mut Q2GameServices, Vec3, Vec3, f64, f64, f64, f64, i32, i32),
    /// Fire a blaster bolt.
    pub fire_blaster:
        fn(ActorId, &mut Q2GameServices, Vec3, Vec3, f64, f64, i64, bool, i32) -> ActorId,
    /// Melee hit.
    pub fire_hit: fn(ActorId, &mut Q2GameServices, Vec3, f64, f64) -> bool,
    /// Fire a rocket.
    pub fire_rocket:
        fn(ActorId, &mut Q2GameServices, Vec3, Vec3, f64, f64, f64, f64) -> ActorId,
    /// Fire a grenade.
    pub fire_grenade: fn(
        ActorId,
        &mut Q2GameServices,
        Vec3,
        Vec3,
        f64,
        f64,
        f64,
        f64,
        bool,
        bool,
        bool,
        Option<Q2GrenadeAdjustment>,
    ) -> ActorId,
    /// Fire a rail slug.
    pub fire_rail: fn(ActorId, &mut Q2GameServices, Vec3, Vec3, f64, f64),
    /// Fire a BFG projectile.
    pub fire_bfg: fn(ActorId, &mut Q2GameServices, Vec3, Vec3, f64, f64, f64) -> ActorId,
}

/// Rogue source-combat hooks (`Q2MonsterSourceCombatHooks`).
pub trait Q2MonsterSourceCombatHooks {
    /// Whether an entity counts as a good guy.
    fn is_good_guy(&mut self, game: &mut Q2GameServices, entity: &Q2Entity) -> bool;
    /// Intercept a damage reaction, reporting whether handled.
    fn before_react(&mut self, context: &mut MonsterContext, attacker: &ActorId) -> bool;
    /// Observe a kill before death processing.
    fn before_killed(&mut self, context: &mut MonsterContext);
    /// Recover an enemy after the current one died.
    fn recover_enemy(&mut self, context: &mut MonsterContext) -> Option<ActorId>;
    /// Intercept a step displacement.
    fn before_move(
        &mut self,
        context: &mut MonsterContext,
        displacement: Vec3,
    ) -> SourceMoveOutcome;
    /// Whether a ground move destination is acceptable.
    fn accepts_ground_move(&mut self, context: &mut MonsterContext, origin: Vec3) -> bool;
    /// Consume a blocked move, reporting whether handled.
    fn consume_blocked(&mut self, context: &mut MonsterContext) -> bool;
}

/// Hint-path hooks (`Q2MonsterHintHooks`).
pub trait Q2MonsterHintHooks {
    /// Run hint-path movement, reporting whether handled.
    fn run(&mut self, context: &mut MonsterContext, distance: f64) -> bool;
    /// Check for a lost hint path.
    fn check_lost(&mut self, context: &mut MonsterContext) -> bool;
    /// Stop hint-path movement.
    fn stop(&mut self, context: &mut MonsterContext);
}

/// Constructor hooks (`Q2MonsterHooks`).
pub struct Q2MonsterHooks {
    /// Drop an authored item.
    pub drop_item: Option<fn(qa_core::identity::OwnedActor, &mut Q2GameServices, &str)>,
    /// Read platform state.
    pub platform_state: Option<fn(&ActorId) -> Option<PlatformPhase>>,
    /// Look up an authored mission.
    pub mission: Option<Box<dyn Fn(&ActorId) -> Option<Box<dyn MonsterMission>>>>,
}

impl Default for Q2MonsterHooks {
    fn default() -> Self {
        Self { drop_item: None, platform_state: None, mission: None }
    }
}

/// Monster context facade (`MonsterContext`).
///
/// A transient borrow of the arena for one monster actor: entity and
/// monster state resolve through the game on every access, so handlers
/// interleave state reads with service calls exactly like the donor.
pub struct MonsterContext<'a> {
    actor: ActorId,
    /// Game services.
    pub game: &'a mut Q2GameServices,
    /// Ballistics bundle.
    pub weapons: MonsterWeapons,
}

impl<'a> MonsterContext<'a> {
    /// Build a context over a monster actor.
    pub fn new(actor: ActorId, game: &'a mut Q2GameServices) -> Self {
        let weapons = game.monsters.weapons;
        Self { actor, game, weapons }
    }

    /// Monster actor.
    pub fn actor(&self) -> &ActorId {
        &self.actor
    }

    /// Monster entity.
    pub fn entity(&self) -> &Q2Entity {
        self.game.require_entity(&self.actor)
    }

    /// Monster entity, mutably.
    pub fn entity_mut(&mut self) -> &mut Q2Entity {
        self.game.require_entity_mut(&self.actor)
    }

    /// Monster state.
    pub fn state(&self) -> &MonsterState {
        self.game.monsters.require_state(&self.actor)
    }

    /// Monster state, mutably.
    pub fn state_mut(&mut self) -> &mut MonsterState {
        self.game.monsters.require_state_mut(&self.actor)
    }

    /// Monster definition.
    pub fn definition(&self) -> std::rc::Rc<Q2MonsterDefinition> {
        self.game.monsters.require_definition(&self.actor)
    }

    /// Monster definition, when admitted.
    pub fn try_definition(&self) -> Option<std::rc::Rc<Q2MonsterDefinition>> {
        self.game.monsters.actor_definitions.get(&self.actor).cloned()
    }

    /// Set the current move (`setMove`).
    pub fn set_move(&mut self, name: &str, immediate: bool) {
        let definition = self.definition();
        let classname = self.entity().classname.clone();
        let movement = definition
            .moves
            .iter()
            .find(|movement| movement.name == name)
            .unwrap_or_else(|| panic!("{classname}: unknown source animation {name}"))
            .clone();
        let rerelease = self.game.options.edition == crate::q2::foundation::host::Q2Edition::Rerelease;
        let state = self.state_mut();
        if rerelease && !immediate {
            state.next_move = Some(movement);
        } else {
            state.current_move = movement;
            state.next_move = None;
        }
    }

    /// Source combat rules (`sourceCombatRules`).
    pub fn source_combat_rules(&self) -> SourceCombatMode {
        self.game.monsters.source_combat
    }

    /// Intercept a step displacement (`beforeSourceMove`).
    ///
    /// The hook object travels outside the arena for the call; hooks are
    /// leaf predicates, so the temporary removal is unobservable.
    pub fn before_source_move(&mut self, displacement: Vec3) -> SourceMoveOutcome {
        let mut hooks = self.game.monsters.source_combat_hooks.take();
        let outcome = hooks
            .as_mut()
            .map(|hooks| hooks.before_move(self, displacement))
            .unwrap_or(SourceMoveOutcome::Move { displacement });
        self.game.monsters.source_combat_hooks = hooks;
        outcome
    }

    /// Whether a ground move destination is acceptable (`acceptsSourceGroundMove`).
    pub fn accepts_source_ground_move(&mut self, origin: Vec3) -> bool {
        let mut hooks = self.game.monsters.source_combat_hooks.take();
        let accepts = hooks
            .as_mut()
            .map(|hooks| hooks.accepts_ground_move(self, origin))
            .unwrap_or(true);
        self.game.monsters.source_combat_hooks = hooks;
        accepts
    }

    /// Consume a blocked move (`consumeSourceBlocked`).
    pub fn consume_source_blocked(&mut self) -> bool {
        let mut hooks = self.game.monsters.source_combat_hooks.take();
        let consumed = hooks
            .as_mut()
            .map(|hooks| hooks.consume_blocked(self))
            .unwrap_or(false);
        self.game.monsters.source_combat_hooks = hooks;
        consumed
    }

    /// Run hint-path movement (`runHintPath`).
    pub fn run_hint_path(&mut self, distance: f64) -> bool {
        let mut hooks = self.game.monsters.hint_hooks.take();
        let handled =
            hooks.as_mut().map(|hooks| hooks.run(self, distance)).unwrap_or(false);
        self.game.monsters.hint_hooks = hooks;
        handled
    }

    /// Check for a lost hint path (`checkLostHintPath`).
    pub fn check_lost_hint_path(&mut self) -> bool {
        let mut hooks = self.game.monsters.hint_hooks.take();
        let lost = hooks.as_mut().map(|hooks| hooks.check_lost(self)).unwrap_or(false);
        self.game.monsters.hint_hooks = hooks;
        lost
    }

    /// Schedule a dead-think callback (`schedule`).
    pub fn schedule(&mut self, delay_seconds: f64, callback: DeadThink) {
        let name = match callback {
            DeadThink::MonsterDeadThink => "monster_dead_think",
            DeadThink::FliesOn => "M_FliesOn",
            DeadThink::FliesOff => "M_FliesOff",
        };
        let think = self.game.source_callbacks.resolve_think(Some(name));
        let think = think.unwrap_or_else(|| panic!("Missing monster thinker {name}"));
        let actor = self.actor.clone();
        self.game.schedule(actor, delay_seconds, think);
    }

    /// Run the stand handler (`stand`).
    pub fn stand(&mut self) {
        let handler = self.definition().stand.clone();
        handler.dispatch(self);
    }

    /// Run the walk handler (`walk`).
    pub fn walk(&mut self) {
        let handler = self.definition().walk.clone();
        handler.dispatch(self);
    }

    /// Run the run handler (`run`).
    pub fn run(&mut self) {
        let handler = self.definition().run.clone();
        handler.dispatch(self);
    }

    /// Run the attack handler (`attack`).
    pub fn attack(&mut self) {
        let handler = self.definition().attack.clone();
        handler.dispatch(self);
    }

    /// Run the melee handler (`melee`).
    pub fn melee(&mut self) {
        let handler = self.definition().melee.clone();
        if let Some(handler) = handler {
            handler.dispatch(self);
        }
    }

    /// Run the idle handler (`idle`).
    pub fn idle(&mut self) {
        let handler = self.definition().idle.clone();
        if let Some(handler) = handler {
            handler.dispatch(self);
        }
    }

    /// Run the search handler (`search`).
    pub fn search(&mut self) {
        let handler = self.definition().search.clone();
        if let Some(handler) = handler {
            handler.dispatch(self);
        }
    }

    /// Dispatch a named callback (`dispatch`).
    pub fn dispatch(&mut self, callback: &str) {
        if callback == "$sight" {
            let handler = self.definition().sight.clone();
            if let Some(handler) = handler {
                handler.dispatch(self);
            }
            return;
        }
        let classname = self.entity().classname.clone();
        let handler = self
            .definition()
            .callbacks
            .get(callback)
            .cloned()
            .or_else(|| super::shared_callback(callback));
        let Some(handler) = handler else {
            panic!("{classname}: unknown source callback {callback}");
        };
        handler.dispatch(self);
    }

    /// Find a target (`findTarget`).
    pub fn find_target(&mut self) -> bool {
        super::perception::find_target(self)
    }

    /// Check for an attack (`checkAttack`).
    pub fn check_attack(&mut self, _distance: f64) -> bool {
        let check = self.definition().check_attack;
        super::perception::check_attack(self, check.unwrap_or(super::perception::default_check_attack))
    }

    /// Move toward the goal (`moveToGoal`).
    pub fn move_to_goal(&mut self, distance: f64) -> bool {
        super::perception::move_to_goal(self, distance)
    }

    /// Dodge an incoming attack (`dodge`).
    pub fn dodge(&mut self, attacker: ActorId, eta_seconds: f64, trace: Option<TraceResult>, gravity: bool) {
        super::monster_dodge(self, attacker, eta_seconds, trace.as_ref(), gravity);
    }

    /// Handle a blocked move (`blocked`).
    pub fn blocked(&mut self, distance: f64) -> bool {
        let blocked = self.definition().blocked;
        if let Some(blocked) = blocked {
            blocked(self, distance)
        } else {
            super::monster_blocked(self, distance)
        }
    }

    /// Read platform state (`platformState`).
    pub fn platform_state(&self, actor: &ActorId) -> Option<PlatformPhase> {
        self.game.monsters.hooks.platform_state.map(|read| read(actor)).flatten()
    }

    /// Look up the authored mission (`hooks.mission`).
    pub fn mission(&self, actor: &ActorId) -> Option<Box<dyn MonsterMission>> {
        self.game.monsters.hooks.mission.as_ref().and_then(|hook| hook(actor))
    }
}

/// Shared power-armor cell store (`bindPowerArmorCells` binding).
///
/// Combat drains land in the shared count; species sync it back to the
/// `q2:monster-power` inventory entry like the donor binding.
#[derive(Debug, Clone, Default)]
pub struct SharedPowerCells(pub std::rc::Rc<std::cell::RefCell<f64>>);

impl PowerArmorCells for SharedPowerCells {
    fn read(&self) -> f64 {
        *self.0.borrow()
    }

    fn write(&mut self, count: f64) {
        *self.0.borrow_mut() = count;
    }
}

/// Bind shared power-armor cells once per actor (`bindPowerArmor` dedup).
pub fn bind_shared_power_cells(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.game.monsters.power_cells.contains_key(&actor) {
        return;
    }
    let cells = std::rc::Rc::new(std::cell::RefCell::new(
        context
            .game
            .host
            .inventory()
            .count(&actor, &"q2:monster-power".to_string()),
    ));
    context
        .game
        .monsters
        .power_cells
        .insert(actor.clone(), cells.clone());
    let owned = context.game.owned_of(actor);
    context
        .game
        .host
        .combat()
        .bind_power_armor_cells(&owned, Box::new(SharedPowerCells(cells)));
}

/// Index a source table with a range error (`recordAt`).
pub fn record_at<T>(values: &[T], index: usize) -> &T {
    values.get(index).unwrap_or_else(|| {
        panic!("Monster source table index {index} outside {}", values.len());
    })
}
