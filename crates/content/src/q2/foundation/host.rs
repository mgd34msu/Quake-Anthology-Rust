//! Q2 gameplay provider boundary (`src/content/q2/foundation/host.ts`).
//!
//! Q2 gameplay logic is adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).
//!
//! [`Q2GameServices`] is the central state arena: it owns the host engine
//! boundary, the entity continuations, the source callback registry and
//! every module runtime. Game logic addresses entities by [`ActorId`]
//! handles; see the [`crate::q2`] module docs for the borrowing rules.

use std::collections::{BTreeMap, HashMap, HashSet};

use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::math::{Bounds, Vec3};

use crate::contract::{ItemId, MonsterDefinitionReference};
use crate::monsters::{AuthoredTarget, MonsterTargetObservation};
use crate::q2::base::entities::BaseEntitiesRuntime;
use crate::q2::base::player::PlayerRuntime;
use crate::q2::equipment::EquipmentRuntime;
use crate::q2::foundation::callbacks::{free_q2_entity, Q2CallbackDefinitions, Q2SourceCallbacks};
use crate::q2::foundation::items::ItemRuntime;
use crate::q2::foundation::monsters::MonsterRuntime;
use crate::q2::foundation::movers::MoverRuntime;
use crate::q2::foundation::shadow_lights::Q2ShadowLightState;
use crate::q2::foundation::weapons::WeaponRuntime;
use crate::q2::missionpacks::items::MissionItemRuntime;
use crate::q2::missionpacks::modes::{DeathballRuntime, TagRuntime};
use crate::q2::missionpacks::monsters::MissionMonsterRuntime;
use crate::q2::multiplayer::ctf::CtfRuntime;
use crate::q2::multiplayer::lmctf::LmctfRuntime;
use crate::q2::rerelease::RereleaseRuntime;
use crate::q2::support::contracts::{
    AttackProvenance, BodyState, DeathReaction, PainReaction, SceneFlare, TouchContact, TraceResult,
    TransitionIntent, WeaponBehaviorProjectilePort, WeaponTrajectoryUpdate,
};
use crate::q2::support::misc::Q2RereleaseRandomSource;
use crate::q2::support::tables::{Q2ActorRegistry, Q2BodyTable, Q2CallbackTable, Q2CombatAuthority, Q2InventoryTable};

/// Q2 edition (`Q2Edition`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2Edition {
    /// Classic game DLL.
    Classic,
    /// Rerelease game DLL.
    Rerelease,
}

/// Q2 game mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2Mode {
    /// Singleplayer.
    Singleplayer,
    /// Cooperative.
    Coop,
    /// Deathmatch.
    Deathmatch,
}

/// Q2 solidity (`Q2Entity["solid"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2Solid {
    /// Not solid.
    None,
    /// Trigger solid.
    Trigger,
    /// Box solid.
    Box,
    /// Brush solid.
    Brush,
}

/// Q2 motion kind (`Q2Motion["kind"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2MotionKind {
    /// Stationary.
    Stationary,
    /// Pusher.
    Push,
    /// Stopped.
    Stop,
    /// Tossed.
    Toss,
    /// New toss.
    NewToss,
    /// Bouncing.
    Bounce,
    /// Wall bouncing.
    WallBounce,
    /// Flying.
    Fly,
    /// Flying missile.
    FlyMissile,
    /// Stepping.
    Step,
}

/// Q2 game options (`Q2GameOptions`).
///
/// `deathmatch_flags` is the boot snapshot; the live value comes from
/// [`Q2GameServices::deathmatch_flags`], which follows the engine hook
/// exactly like the donor's options getter.
#[derive(Debug, Clone)]
pub struct Q2GameOptions {
    /// Edition.
    pub edition: Q2Edition,
    /// Map name.
    pub map_name: String,
    /// Skill level 0-3.
    pub skill: u8,
    /// Game mode.
    pub mode: Q2Mode,
    /// Boot deathmatch flags.
    pub deathmatch_flags: i32,
    /// Maximum clients.
    pub max_clients: u32,
    /// Provider id.
    pub provider: ProviderId,
    /// Damage powerup owner override.
    pub damage_powerup_owner: Option<ProviderId>,
    /// Source damage modifier.
    pub source_damage_modifier: Option<crate::q2::support::contracts::SourceDamageModifier>,
    /// Campaign provider.
    pub campaign: ProviderId,
    /// Combat provider.
    pub combat_provider: ProviderId,
    /// Inventory provider.
    pub inventory_provider: ProviderId,
    /// Movement provider.
    pub movement_provider: ProviderId,
}

/// Authored spawn fields (`Q2SpawnFields`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2SpawnFields {
    /// Authored ordinal.
    pub ordinal: i32,
    /// Classname.
    pub classname: String,
    /// Spawn values in key order (donor insertion order is not observable
    /// here; saves stay deterministic).
    pub values: BTreeMap<String, String>,
}

/// Q2 source counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Q2Counters {
    /// Total secrets.
    pub total_secrets: i32,
    /// Found secrets.
    pub found_secrets: i32,
    /// Total goals.
    pub total_goals: i32,
    /// Found goals.
    pub found_goals: i32,
    /// Total monsters.
    pub total_monsters: i32,
    /// Killed monsters.
    pub killed_monsters: i32,
    /// Server flags.
    pub server_flags: i32,
}

/// Sound loop mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2SoundLoop {
    /// Start looping.
    Start,
    /// Stop looping.
    Stop,
    /// Play once.
    Once,
}

/// Print level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2PrintLevel {
    /// Low priority.
    Low,
    /// Medium priority.
    Medium,
    /// High priority.
    High,
    /// Chat.
    Chat,
}

/// Monster beam effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2MonsterBeam {
    /// Parasite beam.
    Parasite,
    /// Medic beam.
    Medic,
}

/// Q2 presentation event (`Q2PresentationEvent`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2PresentationEvent {
    /// Model presentation.
    Model(Q2ModelEvent),
    /// Visibility presentation.
    Visibility {
        /// Actor.
        actor: ActorId,
        /// Whether visible.
        visible: bool,
    },
    /// Sound presentation.
    Sound(Q2SoundEvent),
    /// Center print.
    CenterPrint {
        /// Actor.
        actor: ActorId,
        /// Text.
        text: String,
        /// Whether instant.
        instant: bool,
        /// Duration in seconds.
        duration_seconds: Option<f64>,
    },
    /// Print.
    Print {
        /// Actor, when targeted.
        actor: Option<ActorId>,
        /// Level.
        level: Q2PrintLevel,
        /// Text.
        text: String,
    },
    /// Help text.
    Help {
        /// Slot 1 or 2.
        slot: u8,
        /// Text.
        text: String,
    },
    /// Light style.
    LightStyle {
        /// Style number.
        style: i32,
        /// Pattern.
        pattern: String,
    },
    /// Music track.
    Music {
        /// Track name.
        track: String,
    },
    /// Effect.
    Effect(Q2EffectEvent),
    /// Damage indicator.
    DamageIndicator {
        /// Actor.
        actor: ActorId,
        /// Origin.
        origin: Vec3,
        /// Amount.
        amount: f64,
    },
    /// Pickup notification.
    Pickup {
        /// Player.
        player: ActorId,
        /// Item.
        item: ItemId,
        /// Icon path.
        icon: String,
        /// Item name.
        name: String,
    },
    /// Point of interest.
    Poi {
        /// Origin.
        origin: Vec3,
        /// Message.
        message: String,
        /// Fields.
        fields: BTreeMap<String, String>,
    },
    /// Dynamic shadow light.
    DynamicLight(Q2ShadowLightState),
    /// Beam.
    Beam(Q2BeamEvent),
    /// Monster beam.
    MonsterBeam {
        /// Effect.
        effect: Q2MonsterBeam,
        /// Actor.
        actor: ActorId,
        /// Start.
        start: Vec3,
        /// End.
        end: Vec3,
    },
    /// Monster muzzle flash.
    MonsterMuzzleflash {
        /// Actor.
        actor: ActorId,
        /// Flash number.
        flash: i32,
        /// Origin.
        origin: Vec3,
        /// Direction.
        direction: Vec3,
    },
    /// Entity event.
    EntityEvent {
        /// Actor.
        actor: ActorId,
        /// Event number.
        event: i32,
    },
}

/// Model presentation event fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2ModelEvent {
    /// Actor.
    pub actor: ActorId,
    /// Model path.
    pub path: String,
    /// Attached models.
    pub attached_models: Vec<String>,
    /// Frame.
    pub frame: i32,
    /// Old frame.
    pub old_frame: i32,
    /// Scale.
    pub scale: f64,
    /// Alpha.
    pub alpha: f64,
    /// Skin.
    pub skin: i32,
    /// Effects.
    pub effects: i64,
    /// Render flags.
    pub render_flags: i32,
}

/// Sound presentation event fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2SoundEvent {
    /// Actor, when attached.
    pub actor: Option<ActorId>,
    /// Origin.
    pub origin: Vec3,
    /// Sound path.
    pub path: String,
    /// Channel.
    pub channel: i32,
    /// Volume.
    pub volume: f64,
    /// Attenuation.
    pub attenuation: f64,
    /// Whether reliable.
    pub reliable: bool,
    /// Loop mode.
    pub loop_: Q2SoundLoop,
    /// Loop owner.
    pub loop_owner: Option<ProviderId>,
}

/// Effect presentation event fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2EffectEvent {
    /// Effect name.
    pub effect: String,
    /// Origin.
    pub origin: Vec3,
    /// Direction.
    pub direction: Vec3,
    /// Count.
    pub count: i32,
    /// Color.
    pub color: i32,
}

/// Beam presentation event fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2BeamEvent {
    /// Actor.
    pub actor: ActorId,
    /// Start.
    pub start: Vec3,
    /// End.
    pub end: Vec3,
    /// Width.
    pub width: f64,
    /// Color.
    pub color: i32,
    /// Whether visible.
    pub visible: bool,
}

/// Q2 trace request (`Q2TraceRequest`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2TraceRequest {
    /// Trace start.
    pub start: Vec3,
    /// Trace end.
    pub end: Vec3,
    /// Trace bounds.
    pub bounds: Option<Bounds>,
    /// Ignored actor.
    pub ignore: Option<ActorId>,
    /// Contents mask.
    pub mask: i32,
    /// Excluded actors.
    pub exclude: Vec<ActorId>,
}

/// Q2 motion record (`Q2Motion`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2Motion {
    /// Actor.
    pub actor: OwnedActor,
    /// Velocity.
    pub velocity: Vec3,
    /// Angular velocity.
    pub angular_velocity: Vec3,
    /// Motion kind.
    pub kind: Q2MotionKind,
    /// Gravity scale.
    pub gravity: f64,
    /// Gravity vector.
    pub gravity_vector: Vec3,
    /// Clip mask.
    pub clip_mask: i32,
    /// Owner.
    pub owner: Option<ActorId>,
}

/// Landmark carry record (`Q2LandmarkCarry`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2LandmarkCarry {
    /// Player.
    pub player: ActorId,
    /// Landmark name.
    pub name: String,
    /// Relative origin.
    pub relative_origin: Vec3,
    /// Relative velocity.
    pub relative_velocity: Vec3,
    /// Relative view angles.
    pub relative_view_angles: Vec3,
}

/// Q2 weapon target record (`Q2WeaponTarget`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Q2WeaponTarget {
    /// Solidity.
    pub solid: Q2Solid,
    /// Laser immunity.
    pub laser_immune: bool,
    /// Damageable target.
    pub damageable_target: bool,
    /// BFG explobox.
    pub bfg_explobox: bool,
}

/// Player view state snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2PlayerViewState {
    /// View angles.
    pub view_angles: Vec3,
    /// Old velocity.
    pub old_velocity: Vec3,
}

/// Think callback (`Q2Think`).
pub type Q2Think = fn(ActorId, &mut Q2GameServices);
/// Use callback (`Q2Use`).
pub type Q2Use = fn(ActorId, &mut Q2GameServices, Option<ActorId>, Option<ActorId>);
/// Touch callback (`Q2Touch`).
pub type Q2Touch = fn(ActorId, &mut Q2GameServices, TouchContact);
/// Pain callback (`Q2Pain`).
pub type Q2Pain = fn(ActorId, &mut Q2GameServices, PainReaction);
/// Die callback (`Q2Die`).
pub type Q2Die = fn(ActorId, &mut Q2GameServices, DeathReaction);
/// Blocked callback (`Q2Entity["blocked"]`).
pub type Q2Blocked = fn(ActorId, &mut Q2GameServices, ActorId);
/// Trajectory projection callback.
pub type Q2TouchProject = fn(ActorId, &mut Q2GameServices, &WeaponTrajectoryUpdate);
/// Spawn callback (`Q2SpawnModule["spawn"]`).
pub type Q2SpawnFn = fn(ActorId, &mut Q2GameServices) -> bool;
/// Item-name callback (`Q2SpawnModule["itemName"]`).
pub type Q2ItemNameFn = fn(&str) -> Option<String>;

/// Original-pickup continuation (`{ eligible, original, complete }` in
/// `items.ts`). The narrow object-safe boundary over the engine's
/// original-pickup admission: the trait object cannot hold the donor's
/// capturing closures, so the continuation carries the pickup data and
/// receives services on each call.
pub trait Q2OriginalPickupContinuation {
    /// Whether the pickup is eligible.
    fn eligible(&mut self, game: &mut Q2GameServices) -> bool;
    /// Run the original grant, reporting whether taken.
    fn original(&mut self, game: &mut Q2GameServices) -> bool;
    /// Complete the pickup.
    fn complete(&mut self, game: &mut Q2GameServices, taken: bool);
}

/// Original-pickup admission boundary (donor `OriginalPickupAdmission`
/// as used by `items.ts`). The engine adapts its admission policy here.
pub trait Q2OriginalPickups {
    /// Touch a pickup through the original admission policy.
    fn touch(
        &mut self,
        offer: &crate::contract::OriginalPickupOffer,
        continuation: &mut dyn Q2OriginalPickupContinuation,
    );
}

/// Live deathmatch-flags cell (`Q2CompositionServices["deathmatchFlags"]`).
pub trait DeathmatchFlags {
    /// Read the live flags.
    fn read(&self) -> i32;
    /// Write the live flags.
    fn write(&mut self, flags: i32);
}

/// Foreign monster admission (`Q2Foundation["monsterAdmission"]`).
/// Set by the application, which resolves foreign monster classnames to
/// definition references and admits them.
pub struct MonsterAdmission {
    /// Resolve a classname to a foreign definition reference.
    pub resolve: fn(&str, &Q2SpawnFields) -> Option<MonsterDefinitionReference>,
    /// Admit a foreign monster actor.
    pub spawn: fn(&mut Q2GameServices, &OwnedActor, &Q2SpawnFields, &MonsterDefinitionReference),
}

/// Active match mode (composition selection without payloads; payloads
/// live in the per-mode runtimes and are set by composition).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ActiveMatch {
    /// Standard play.
    #[default]
    Standard,
    /// Capture the flag.
    Ctf,
    /// LMCTF.
    Lmctf,
    /// Tag.
    Tag,
    /// Deathball.
    Deathball,
}

/// Q2 foundation host boundary (`Q2FoundationHost`).
pub trait Q2FoundationHost {
    /// Session actor registry.
    fn actors(&mut self) -> &mut dyn Q2ActorRegistry;
    /// Shared body table.
    fn bodies(&mut self) -> &mut dyn Q2BodyTable;
    /// Actor callback table.
    fn callbacks(&mut self) -> &mut dyn Q2CallbackTable;
    /// Gameplay combat authority.
    fn combat(&mut self) -> &mut dyn Q2CombatAuthority;
    /// Shared inventory table.
    fn inventory(&mut self) -> &mut dyn Q2InventoryTable;
    /// Original-pickup admission, when the engine provides one.
    fn original_pickups(&mut self) -> Option<&mut dyn Q2OriginalPickups> {
        None
    }
    /// Weapon behavior port, when the engine provides one.
    fn weapon_behavior(&mut self) -> Option<&mut dyn WeaponBehaviorProjectilePort> {
        None
    }
    /// Weapon-target override. `None` selects the default record; the
    /// engine returns `Some` (even `Some(None)`) to override it, exactly
    /// like the donor's optional method.
    fn weapon_target_override(&mut self, _actor: &ActorId) -> Option<Option<Q2WeaponTarget>> {
        None
    }
    /// Monster-target override, following the same rule.
    fn monster_target_override(&mut self, _actor: &ActorId) -> Option<Option<MonsterTargetObservation>> {
        None
    }
    /// Record an entity continuation. The engine uses its own game handle
    /// for services; the donor's services parameter cannot cross the
    /// boundary.
    fn register_entity(&mut self, _actor: &OwnedActor) {}
    /// Current source time in seconds.
    fn now(&self) -> f64;
    /// Frame time in seconds.
    fn frame_seconds(&self) -> f64;
    /// Gravity.
    fn gravity(&self) -> f64;
    /// Draw a unit random number.
    fn random(&mut self) -> f64;
    /// Rerelease random stream; classic hosts omit it.
    fn rerelease_random(&mut self) -> Option<&mut dyn Q2RereleaseRandomSource> {
        None
    }
    /// Schedule an actor's bound think callback.
    fn schedule(&mut self, actor: &OwnedActor, due_seconds: Option<f64>);
    /// Touch triggers for an actor.
    fn touch_triggers(&mut self, actor: &OwnedActor);
    /// Trace.
    fn trace(&mut self, request: &Q2TraceRequest) -> TraceResult;
    /// Point contents.
    fn point_contents(&mut self, point: Vec3) -> i32;
    /// Potential visibility check.
    fn in_pvs(&mut self, first: Vec3, second: Vec3) -> bool;
    /// Potential hearing check.
    fn in_phs(&mut self, first: Vec3, second: Vec3) -> bool;
    /// Area connectivity check.
    fn areas_connected(&mut self, first: Vec3, second: Vec3) -> bool;
    /// Nearby actors in source-stable traversal order.
    fn nearby(&mut self, origin: Vec3, radius: f64) -> Vec<ActorId>;
    /// Player actors.
    fn players(&mut self) -> Vec<ActorId>;
    /// World actor.
    fn world_actor(&mut self) -> ActorId;
    /// Whether an actor is a player.
    fn is_player(&mut self, actor: &ActorId) -> bool;
    /// Whether an actor is a monster.
    fn is_monster(&mut self, actor: &ActorId) -> bool;
    /// Inline model bounds.
    fn inline_model_bounds(&mut self, model: i32) -> Bounds;
    /// Set solidity.
    fn set_solid(&mut self, actor: &OwnedActor, solid: Q2Solid, model: Option<i32>);
    /// Set motion.
    fn set_motion(&mut self, motion: &Q2Motion);
    /// Set an area portal.
    fn set_area_portal(&mut self, portal: i32, open: bool);
    /// Emit a presentation event.
    fn emit(&mut self, event: Q2PresentationEvent);
    /// Player view state.
    fn player_view_state(&mut self, player: &ActorId) -> Option<Q2PlayerViewState>;
    /// Consume a key for a player.
    fn key_consumed(&mut self, player: &ActorId);
    /// Capture campaign state before travel.
    fn prepare_level_change(&mut self, map: &str, landmark: Option<&Q2LandmarkCarry>, server_flags: i32);
    /// Propose a transition intent.
    fn transition(&mut self, intent: TransitionIntent);
    /// Emit a diagnostic.
    fn diagnostic(&mut self, message: &str);
}

/// Spawn module entry (`Q2SpawnModule`).
#[derive(Clone)]
pub struct SpawnModule {
    /// Spawn handler.
    pub spawn: Q2SpawnFn,
    /// Item-name handler.
    pub item_name: Q2ItemNameFn,
    /// Callback definitions.
    pub callbacks: Q2CallbackDefinitions,
}

/// Q2 entity (`Q2Entity`).
///
/// Provider-owned fields exclude shared body, health, armor and inventory
/// state. Callback slots hold `fn` pointers; entities are addressed by
/// [`ActorId`] handles from the arena.
#[derive(Debug, Clone)]
pub struct Q2Entity {
    /// Owning actor.
    pub actor: OwnedActor,
    /// Spawn fields.
    pub spawn: Q2SpawnFields,
    /// Classname.
    pub classname: String,
    /// Trigger target.
    pub target: String,
    /// Target name.
    pub targetname: String,
    /// Kill target.
    pub killtarget: String,
    /// Combat target.
    pub combat_target: String,
    /// Death target.
    pub death_target: String,
    /// Health target.
    pub health_target: String,
    /// Item target.
    pub item_target: String,
    /// Message.
    pub message: String,
    /// Model path.
    pub model: String,
    /// Attached model 2.
    pub model2: String,
    /// Attached model 3.
    pub model3: String,
    /// Attached model 4.
    pub model4: String,
    /// Spawn flags.
    pub spawnflags: i32,
    /// Delay seconds.
    pub delay: f64,
    /// Wait seconds.
    pub wait: f64,
    /// Speed.
    pub speed: f64,
    /// Acceleration.
    pub accel: f64,
    /// Deceleration.
    pub decel: f64,
    /// Damage.
    pub damage: f64,
    /// Damage radius.
    pub damage_radius: f64,
    /// Radius damage.
    pub radius_damage: f64,
    /// Count.
    pub count: i32,
    /// Maximum health.
    pub max_health: f64,
    /// View height.
    pub view_height: i32,
    /// Frame.
    pub frame: i32,
    /// Old frame.
    pub old_frame: i32,
    /// Scale.
    pub scale: f64,
    /// Alpha.
    pub alpha: f64,
    /// Skin.
    pub skin: i32,
    /// Effects.
    pub effects: i64,
    /// Render flags.
    pub render_flags: i32,
    /// Flags.
    pub flags: i64,
    /// Server flags.
    pub server_flags: i32,
    /// Light level.
    pub light_level: i32,
    /// Power cubes.
    pub power_cubes: i32,
    /// Timestamp.
    pub timestamp: f64,
    /// Noise path.
    pub noise: String,
    /// Sound path.
    pub sound: String,
    /// Volume.
    pub volume: f64,
    /// Attenuation.
    pub attenuation: f64,
    /// Random variance.
    pub random: f64,
    /// Map name.
    pub map: String,
    /// Style.
    pub style: i32,
    /// Whether a transition started.
    pub transition_started: bool,
    /// Clip mask.
    pub clip_mask: i32,
    /// Whether a projectile.
    pub projectile: bool,
    /// Whether dodgeable.
    pub dodgeable: bool,
    /// Laser immunity.
    pub laser_immune: bool,
    /// Damageable target.
    pub damageable_target: bool,
    /// Last attack provenance.
    pub last_attack: Option<AttackProvenance>,
    /// Whether visible.
    pub visible: bool,
    /// Solidity.
    pub solid: Q2Solid,
    /// Motion kind.
    pub motion: Q2MotionKind,
    /// Gravity scale.
    pub gravity: f64,
    /// Gravity vector.
    pub gravity_vector: Vec3,
    /// Angular velocity.
    pub angular_velocity: Vec3,
    /// Move direction.
    pub movedir: Vec3,
    /// Position 1.
    pub pos1: Vec3,
    /// Position 2.
    pub pos2: Vec3,
    /// Activator.
    pub activator: Option<ActorId>,
    /// Enemy.
    pub enemy: Option<ActorId>,
    /// Owner.
    pub owner: Option<ActorId>,
    /// Goal.
    pub goal: Option<ActorId>,
    /// Team master.
    pub team_master: Option<ActorId>,
    /// Team chain.
    pub team_chain: Option<ActorId>,
    /// Chain.
    pub chain: Option<ActorId>,
    /// Beam.
    pub beam: Option<ActorId>,
    /// Second beam.
    pub beam2: Option<ActorId>,
    /// Proboscis.
    pub proboscus: Option<ActorId>,
    /// Next think time.
    pub next_think: Option<f64>,
    /// Think callback.
    pub think: Option<Q2Think>,
    /// Pre-physics callback.
    pub prethink: Option<Q2Think>,
    /// Post-physics callback.
    pub postthink: Option<Q2Think>,
    /// Use callback.
    pub use_: Option<Q2Use>,
    /// Touch callback.
    pub touch: Option<Q2Touch>,
    /// Pain callback.
    pub pain: Option<Q2Pain>,
    /// Die callback.
    pub die: Option<Q2Die>,
    /// Blocked callback.
    pub blocked: Option<Q2Blocked>,
}

impl Q2Entity {
    /// Build an entity continuation over an owned actor.
    pub fn new(actor: OwnedActor, spawn: Q2SpawnFields) -> Self {
        let field = |key: &str| spawn.values.get(key).cloned().unwrap_or_default();
        Self {
            classname: spawn.classname.clone(),
            target: field("target"),
            targetname: field("targetname"),
            killtarget: field("killtarget"),
            combat_target: field("combattarget"),
            death_target: field("deathtarget"),
            health_target: field("healthtarget"),
            item_target: field("itemtarget"),
            message: field("message"),
            model: field("model"),
            model2: String::new(),
            model3: String::new(),
            model4: String::new(),
            spawnflags: 0,
            delay: 0.0,
            wait: 0.0,
            speed: 0.0,
            accel: 0.0,
            decel: 0.0,
            damage: 0.0,
            damage_radius: 0.0,
            radius_damage: 0.0,
            count: 0,
            max_health: 0.0,
            view_height: 0,
            frame: 0,
            old_frame: -1,
            scale: 1.0,
            alpha: 1.0,
            skin: 0,
            effects: 0,
            render_flags: 0,
            flags: 0,
            server_flags: 0,
            light_level: 128,
            power_cubes: 0,
            timestamp: 0.0,
            noise: String::new(),
            sound: String::new(),
            volume: 0.0,
            attenuation: 0.0,
            random: 0.0,
            map: String::new(),
            style: 0,
            transition_started: false,
            clip_mask: 0x6000003,
            projectile: false,
            dodgeable: false,
            laser_immune: false,
            damageable_target: false,
            last_attack: None,
            visible: true,
            solid: Q2Solid::None,
            motion: Q2MotionKind::Stationary,
            gravity: 1.0,
            gravity_vector: Vec3 { x: 0.0, y: 0.0, z: -1.0 },
            angular_velocity: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            movedir: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            pos1: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            pos2: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            activator: None,
            enemy: None,
            owner: None,
            goal: None,
            team_master: None,
            team_chain: None,
            chain: None,
            beam: None,
            beam2: None,
            proboscus: None,
            next_think: None,
            think: None,
            prethink: None,
            postthink: None,
            use_: None,
            touch: None,
            pain: None,
            die: None,
            blocked: None,
            actor,
            spawn,
        }
    }

    /// Scene flare record (`Q2Entity["flare"]`).
    pub fn flare(&self) -> Option<SceneFlare> {
        use crate::q2::foundation::fields::number_field;

        if self.render_flags & 0x200000 == 0 {
            return None;
        }
        let shell = self.render_flags & 0x1c00;
        let image = if self.render_flags & 256 != 0 {
            self.spawn.values.get("image").cloned().unwrap_or_else(|| "misc/flare.tga".to_string())
        } else {
            "misc/flare.tga".to_string()
        };
        let color = if self.skin == 0 {
            Vec3 { x: 255.0, y: 255.0, z: 255.0 }
        } else {
            let bits = self.skin as u32;
            Vec3 {
                x: (bits >> 24) as f32,
                y: (bits >> 16 & 255) as f32,
                z: (bits >> 8 & 255) as f32,
            }
        };
        Some(SceneFlare {
            image,
            fade_start: number_field(&self.spawn, "fade_start_dist", 96.0).trunc(),
            fade_end: number_field(&self.spawn, "fade_end_dist", 384.0).trunc(),
            scale: if self.scale == 0.0 { 1.0 } else { self.scale },
            lock_angle: self.render_flags & 1 != 0,
            color,
            rim_color: if shell == 0 {
                None
            } else {
                Some(Vec3 {
                    x: if shell & 0x400 != 0 { 255.0 } else { 0.0 },
                    y: if shell & 0x800 != 0 { 255.0 } else { 0.0 },
                    z: if shell & 0x1000 != 0 { 255.0 } else { 0.0 },
                })
            },
        })
    }

    /// Authored target view (`useTargets` takes `AuthoredTarget`).
    pub fn authored_target(&self) -> AuthoredTarget {
        AuthoredTarget {
            actor: self.actor.clone(),
            classname: self.classname.clone(),
            targetname: self.targetname.clone(),
            target: self.target.clone(),
            delay: self.delay,
            message: self.message.clone(),
            killtarget: self.killtarget.clone(),
        }
    }
}

/// Q2 game services arena (`Q2GameServices` / `Q2EntityServices`).
///
/// Owns the host boundary, entity continuations, the source callback
/// registry and every module runtime. All fields are public so module
/// logic reads state directly; mutation still flows through the narrow
/// service methods below plus each module's functions.
pub struct Q2GameServices {
    /// Engine host boundary.
    pub host: Box<dyn Q2FoundationHost>,
    /// Game options.
    pub options: Q2GameOptions,
    /// Entity continuations by actor.
    pub entities: HashMap<ActorId, Q2Entity>,
    /// Foreign authored targets.
    pub authored_targets: HashMap<ActorId, AuthoredTarget>,
    /// Source callback registry.
    pub source_callbacks: Q2SourceCallbacks,
    /// Source counters.
    pub counters: Q2Counters,
    /// Spawn modules in dispatch order.
    pub modules: Vec<SpawnModule>,
    /// Next source slot.
    pub next_source_slot: u32,
    /// Source slot by actor.
    pub source_slots: HashMap<ActorId, u32>,
    /// Freed source slots with release times.
    pub freed_slots: HashMap<u32, f64>,
    /// Attack sequence.
    pub sequence: u64,
    /// Actor inside a callback boundary.
    pub current_actor: Option<ActorId>,
    /// Unsupported entities by actor.
    pub unsupported: HashSet<ActorId>,
    /// Foreign monster admission.
    pub monster_admission: Option<MonsterAdmission>,
    /// Live deathmatch-flags cell.
    pub deathmatch_flags: Option<Box<dyn DeathmatchFlags>>,
    /// Active match mode.
    pub match_mode: ActiveMatch,
    /// Monster runtime.
    pub monsters: MonsterRuntime,
    /// Weapon runtime.
    pub weapons: WeaponRuntime,
    /// Item runtime.
    pub items: ItemRuntime,
    /// Mover runtime.
    pub movers: MoverRuntime,
    /// Player runtime.
    pub players: PlayerRuntime,
    /// Base entity runtime.
    pub base_entities: BaseEntitiesRuntime,
    /// Mission-pack monster runtime.
    pub mission_monsters: MissionMonsterRuntime,
    /// Mission-pack item runtime.
    pub mission_items: MissionItemRuntime,
    /// Tag runtime.
    pub tag: TagRuntime,
    /// Deathball runtime.
    pub deathball: DeathballRuntime,
    /// CTF runtime.
    pub ctf: CtfRuntime,
    /// LMCTF runtime.
    pub lmctf: LmctfRuntime,
    /// Rerelease runtime.
    pub rerelease: RereleaseRuntime,
    /// Equipment runtime.
    pub equipment: EquipmentRuntime,
}

impl Q2GameServices {
    /// Build game services over a host.
    pub fn new(host: Box<dyn Q2FoundationHost>, options: Q2GameOptions, modules: Vec<SpawnModule>) -> Self {
        let mut source_callbacks = Q2SourceCallbacks::new();
        let mut builtin = Q2CallbackDefinitions::default();
        builtin.think.insert("Think_Delay", super::entity_services::delayed_use as Q2Think);
        builtin.think.insert("G_FreeEdict", free_q2_entity as Q2Think);
        builtin.die.insert("G_FreeEdict", super::entity_services::free_q2_entity_die as Q2Die);
        source_callbacks.register(&builtin);
        for module in &modules {
            source_callbacks.register(&module.callbacks);
        }
        let next_source_slot = options.max_clients + 1;
        Self {
            host,
            options,
            entities: HashMap::new(),
            authored_targets: HashMap::new(),
            source_callbacks,
            counters: Q2Counters::default(),
            modules,
            next_source_slot,
            source_slots: HashMap::new(),
            freed_slots: HashMap::new(),
            sequence: 0,
            current_actor: None,
            unsupported: HashSet::new(),
            monster_admission: None,
            deathmatch_flags: None,
            match_mode: ActiveMatch::default(),
            monsters: MonsterRuntime::default(),
            weapons: WeaponRuntime::default(),
            items: ItemRuntime::default(),
            movers: MoverRuntime::default(),
            players: PlayerRuntime::default(),
            base_entities: BaseEntitiesRuntime::default(),
            mission_monsters: MissionMonsterRuntime::default(),
            mission_items: MissionItemRuntime::default(),
            tag: TagRuntime::default(),
            deathball: DeathballRuntime::default(),
            ctf: CtfRuntime::default(),
            lmctf: LmctfRuntime::default(),
            rerelease: RereleaseRuntime::default(),
            equipment: EquipmentRuntime::default(),
        }
    }

    /// Live deathmatch flags (follows the engine hook when present).
    pub fn deathmatch_flags(&self) -> i32 {
        self.deathmatch_flags.as_ref().map_or(self.options.deathmatch_flags, |flags| flags.read())
    }

    /// Look up an entity continuation (`entity`).
    ///
    /// The donor resolves through the registry first; the arena cleans
    /// continuations synchronously on every release (game-initiated
    /// directly, foreign releases through `on_actor_released`), and
    /// [`ActorId`] equality covers the session token plus slot and
    /// generation, so a plain map lookup is equivalent.
    pub fn entity(&self, actor: &ActorId) -> Option<&Q2Entity> {
        self.entities.get(actor)
    }

    /// Mutably borrow an entity continuation.
    pub fn entity_mut(&mut self, actor: &ActorId) -> Option<&mut Q2Entity> {
        self.entities.get_mut(actor)
    }

    /// Require an entity continuation (callback invariant).
    pub fn require_entity(&self, actor: &ActorId) -> &Q2Entity {
        self.entities
            .get(actor)
            .unwrap_or_else(|| panic!("Q2 callback used a missing entity {}", actor.slot()))
    }

    /// Mutably require an entity continuation (callback invariant).
    pub fn require_entity_mut(&mut self, actor: &ActorId) -> &mut Q2Entity {
        self.entities
            .get_mut(actor)
            .unwrap_or_else(|| panic!("Q2 callback used a missing entity {}", actor.slot()))
    }

    /// Read a shared body (`body`).
    pub fn body_of(&mut self, actor: ActorId) -> BodyState {
        self.host
            .bodies()
            .read(&actor)
            .unwrap_or_else(|| panic!("Q2 callback used a released body"))
    }

    /// Write a shared body and optionally link it (`move`).
    pub fn write_body(&mut self, actor: ActorId, state: &BodyState, link: bool) {
        let owned = self.owned_of(actor);
        self.host.bodies().write(&owned, state);
        if link {
            self.host.bodies().link(&owned, None);
        }
    }

    /// Link an entity body (`link`).
    pub fn link_actor(&mut self, actor: ActorId) {
        let owned = self.owned_of(actor);
        self.host.bodies().link(&owned, None);
    }

    /// Emit a presentation event.
    pub fn host_emit(&mut self, event: Q2PresentationEvent) {
        self.host.emit(event);
    }

    /// Owned handle for an arena entity.
    pub fn owned_of(&self, actor: ActorId) -> OwnedActor {
        self.entities
            .get(&actor)
            .unwrap_or_else(|| panic!("Q2 callback used a missing entity {}", actor.slot()))
            .actor
            .clone()
    }

    /// Current source time.
    pub fn now(&self) -> f64 {
        self.host.now()
    }

    /// Draw a unit random number.
    pub fn random(&mut self) -> f64 {
        self.host.random()
    }
}
