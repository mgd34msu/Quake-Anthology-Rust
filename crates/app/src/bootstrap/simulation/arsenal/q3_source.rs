//! Original Q3 weapon/equipment records over actors admitted by their world.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/arsenal/q3-source.ts`
//! (`Q3SelectedClientPose`, `Q3SelectedSourceHost`, `Q3SelectedClientEffects`,
//! `Q3SelectedEquipmentState`, `Q3SelectedSource`).
//!
//! The Rust content Q3 game models (`game/entities.rs`, `game/items_core.rs`,
//! `game/state.rs`, `team_arena/support.rs`) are self-contained mirrors that do
//! not share identity or entity types with the app-integrated records model
//! (`base/records.rs`, `base/world_adapter.rs`, `base/combat_bridge.rs`), so the
//! game-simulation calls (missiles, weapons, portals, pickups, holdables,
//! deaths, team-arena effects, save graphs, presentation) cross into the
//! runtime-owned backend through [`Q3SourceGame`]. Every seam method cites its
//! donor call and the Rust reason. Records, slots, actors, publishing, saves,
//! and the owner state machine below are real.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use qa_content::contract::{ContentId, ItemId, OriginalPickupOffer, PickupResource, ProtectionChannel};
use qa_content::q2::foundation::host::{Q2Motion, Q2MotionKind};
use qa_content::q3::base::combat_bridge::{
    Q3CombatBridge, Q3CombatBridgeHost, SourceDamageModifier, VictimArmorContext,
};
use qa_content::q3::base::game::item_lifecycle::{SourcePickupAdmission, SourcePickupDescriptor};
use qa_content::q3::base::game::numeric::GameRandom;
use qa_content::q3::base::game::state::MAX_CLIENTS;
use qa_content::q3::base::game::utilities::ConfigStringStore;
use qa_content::q3::base::records::{
    AttackCause, DamageDecision, DamageRequest, EntityRef, Q3ActorCallbacks, Q3BaseError, Q3DamageCall, Q3EntityPool,
    Q3EntityRecords, Q3RecordHost, Q3SessionActors, Q3SessionBodies, Q3SessionCombat, Q3SessionInventory,
};
use qa_content::q3::base::records::{ClientBackingSnapshot, ClientRef, GameClient};
use qa_content::q3::base::shared::definitions::{
    EntityType, ItemType, PersistentIndex, Powerup, Product, StatSchema, Team,
};
use qa_content::q3::base::shared::entity_shared::EntityCollisionModel;
use qa_content::q3::base::shared::entity_state::EntityState;
use qa_content::q3::base::shared::items::{
    can_item_be_grabbed, can_q3_armor_be_grabbed, item_at, item_list, PickupEntity,
    PlayerInventory as SharedPlayerInventory,
};
use qa_content::q3::base::shared::player_state::AuthorityStores;
use qa_content::q3::base::shared::player_state::PlayerState;
use qa_content::q3::base::shared::snapshot_state::player_state_to_entity_state;
use qa_content::q3::base::shared::trajectory::{Trajectory, TrajectoryType};
use qa_content::q3::base::world_adapter::{
    ActorCollision, Q3TraceQuery, Q3TraceResult, Q3WorldAdapter, Q3WorldAdapterHost,
};
use qa_content::q3::team_arena::client_admission::{clean_client_name, client_info_value};
use qa_content::q3::team_arena::client_effects::Q3MappedAmmoTimer;
use qa_core::identity::{ActorId, OwnedActor, ProviderId, SavedActorId};
use qa_core::math::Bounds;
use qa_core::math::Vec3;
use qa_world::body::BodyState;
use qa_world::movement::drop_q3_movement_timers;
use qa_world::movement::q3::constants::move_flags::{GRAPPLE_PULL, INVULEXPAND};
use qa_world::movement::q3::postures::q3_invulnerability_pose;
use qa_world::movement::q3::types::Q3Postures;
use qa_world::movement::q3::weapon::q3_weapon_delay;
use qa_world::movement::types::FixedMovementPose;
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::value::{arr, int, obj, str, SaveJson, SaveReader};
use qa_world::WorldError;

use super::super::q3::types::Q3SourcePresentationState;
use super::super::q3_ballistics::{Q3ProjectileState, Q3WeaponStatistics};
use super::super::types::SimulationPresentation;
use super::selected::WeaponStepInput;

/// Errors in the selected Q3 source owner.
#[derive(Debug, thiserror::Error)]
pub enum Q3SourceError {
    /// Selected Q3 think actor is retired.
    #[error("Selected Q3 think actor is retired")]
    RetiredThink,
    /// NULL ent->think.
    #[error("NULL ent->think")]
    NullThink,
    /// Q3 source projection requires the existing actor body.
    #[error("Q3 source projection requires the existing actor body")]
    MissingBody,
    /// Selected Q3 source projection capacity exceeded.
    #[error("Selected Q3 source projection capacity exceeded")]
    ProjectionCapacity,
    /// Selected Q3 world actor changed its source slot.
    #[error("Selected Q3 world actor changed its source slot")]
    WorldSlotChanged,
    /// Selected Q3 source is closed.
    #[error("Selected Q3 source is closed")]
    Closed,
    /// Selected Q3 motion actor is retired.
    #[error("Selected Q3 motion actor is retired")]
    RetiredMotion,
    /// Selected Q3 collision actor is retired.
    #[error("Selected Q3 collision actor is retired")]
    RetiredCollision,
    /// Selected Q3 pose has no source client.
    #[error("Selected Q3 pose has no source client")]
    MissingPoseClient,
    /// Selected Q3 movement clock has no source client.
    #[error("Selected Q3 movement clock has no source client")]
    MissingMovementClient,
    /// Selected Q3 arsenal requires an admitted player.
    #[error("Selected Q3 arsenal requires an admitted player")]
    MissingPlayer,
    /// Missing selected Q3 client.
    #[error("Missing selected Q3 client")]
    MissingClient,
    /// Selected firing delay requires its source client.
    #[error("Selected firing delay requires its source client")]
    MissingDelayClient,
    /// Original equipment pickup requires a source client.
    #[error("Original equipment pickup requires a source client")]
    MissingPickupClient,
    /// Selected Q3 holdable consumption differs from its current source item.
    #[error("Selected Q3 holdable consumption differs from its current source item")]
    HoldableMismatch,
    /// Restored selected equipment has no source client.
    #[error("Restored selected equipment has no source client")]
    MissingRestoredClient,
    /// Restored selected holdable differs from its source item.
    #[error("Restored selected holdable differs from its source item")]
    RestoredHoldableMismatch,
    /// Cannot save selected Q3 during a source call.
    #[error("Cannot save selected Q3 during a source call")]
    SaveDuringCall,
    /// Legacy Q3 restore requires an unused base source owner.
    #[error("Legacy Q3 restore requires an unused base source owner")]
    LegacyRequiresUnused,
    /// Legacy Q3 player arsenal differs from its source.
    #[error("Legacy Q3 player arsenal differs from its source")]
    LegacyArsenalMismatch,
    /// Legacy Q3 player lost its source client.
    #[error("Legacy Q3 player lost its source client")]
    LegacyMissingClient,
    /// Selected Q3 restore requires an unused source owner.
    #[error("Selected Q3 restore requires an unused source owner")]
    RestoreRequiresUnused,
    /// Cannot close selected Q3 during a source call.
    #[error("Cannot close selected Q3 during a source call")]
    CloseDuringCall,
    /// Imported Q3 projectile disappeared.
    #[error("Imported Q3 projectile disappeared")]
    LegacyImportLost,
    /// Legacy Q3 arsenal does not match its original base supply profile.
    #[error("Legacy Q3 arsenal does not match its original base supply profile")]
    LegacyProfileMismatch,
    /// Legacy ballistics require the base Q3 source.
    #[error("Legacy ballistics require the base Q3 source")]
    LegacyRequiresBase,
    /// Legacy projectile has no unique owned body.
    #[error("Legacy projectile has no unique owned body")]
    LegacyProjectileBody,
    /// Only a legacy grappling hook can be attached.
    #[error("Only a legacy grappling hook can be attached")]
    LegacyAttachedHook,
    /// Legacy grappling hook has no unique live player.
    #[error("Legacy grappling hook has no unique live player")]
    LegacyHookPlayer,
    /// Legacy Q3 client continuation has no unique live player.
    #[error("Legacy Q3 client continuation has no unique live player")]
    LegacyClientContinuations,
    /// Legacy Q3 reference has no body.
    #[error("Legacy Q3 reference has no body")]
    LegacyReferenceBody,
    /// Legacy Q3 projectiles exceed original source entity capacity.
    #[error("Legacy Q3 projectiles exceed original source entity capacity")]
    LegacyCapacity,
    /// Wrapped world/save error.
    #[error(transparent)]
    World(#[from] WorldError),
    /// Wrapped Q3 base error.
    #[error(transparent)]
    Base(#[from] Q3BaseError),
}

/// Equipment ownership of the selected source (donor `equipment.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3EquipmentKind {
    /// The source owns equipment.
    Source,
    /// The primary owns equipment.
    Primary,
}

/// Physics family tag: T-mirror of donor `PhysicsFamily` in
/// `src/app/bootstrap/simulation/physics.ts` (`"q1" | "q2" | "q3"`).
/// Unify with the sim-physics lane's port when it lands; until then this
/// module owns the shape for `Q3SelectedSource::collision`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicsFamily {
    /// Quake.
    Q1,
    /// Quake II.
    Q2,
    /// Quake III.
    Q3,
}

/// Shared solidity record: T-mirror of donor `SharedSolid` in
/// `src/app/bootstrap/simulation/physics.ts`. Unify with the sim-physics
/// lane's port when it lands.
#[derive(Debug, Clone, PartialEq)]
pub struct SharedSolid {
    /// Solidity.
    pub solid: SharedSolidKind,
    /// Inline model index, if any.
    pub model: Option<i32>,
    /// Physics family.
    pub family: PhysicsFamily,
    /// Owning actor, if any.
    pub owner: Option<ActorId>,
    /// Monster flag.
    pub monster: bool,
    /// Dead-monster flag.
    pub dead_monster: bool,
    /// Quake corpse flag.
    pub q1_corpse: bool,
    /// Item flag.
    pub item: bool,
}

/// Solidity word of [`SharedSolid`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedSolidKind {
    /// Not solid.
    None,
    /// Trigger solid.
    Trigger,
    /// Box solid.
    Box,
    /// Brush solid.
    Brush,
}

/// Current destination pose only; source equipment and timers remain in
/// their original private records (donor `Q3SelectedClientPose`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SelectedClientPose {
    /// View angles.
    pub angles: Vec3,
    /// View height.
    pub view_height: i32,
    /// Maximum health.
    pub max_health: i32,
    /// Team.
    pub team: Team,
    /// Quad expiry time in milliseconds.
    pub quad_until: i32,
    /// Haste expiry time in milliseconds.
    pub haste_until: i32,
}

/// Copied client effect words (donor `Q3SelectedClientEffects`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SelectedClientEffects {
    /// Teleport bit.
    pub teleport_bit: i32,
    /// View angles.
    pub view_angles: Vec3,
    /// Delta angles.
    pub delta_angles: Vec3,
    /// Player-move flags.
    pub pm_flags: i32,
    /// Player-move time.
    pub pm_time: i32,
    /// Invulnerability expiry time.
    pub invulnerability_time: i32,
    /// Maximum health.
    pub max_health: i32,
}

/// Equipment-owned runtime slice (donor `Q3SelectedEquipmentState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3SelectedEquipmentState {
    /// Maximum health.
    pub max_health: i32,
    /// Persistent powerup tag.
    pub persistent_powerup_tag: i32,
    /// Holdable item index.
    pub holdable_item: i32,
    /// Holdable tag.
    pub holdable_tag: i32,
}

/// Spawn point (donor `spawnPoint` return).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3SpawnPoint {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
}

/// Weapon effects overrides (donor `Q3SelectedSourceHost["weaponEffects"]`).
pub trait Q3SourceWeaponEffects {
    /// Damage factor for an actor.
    fn damage_factor(&self, actor: &ActorId) -> f32;
    /// Firing delay for an actor in milliseconds.
    fn firing_delay(&self, actor: &ActorId, milliseconds: i32, persistent_powerup: i32) -> i32;
}

/// Selected Q3 source host (donor `Q3SelectedSourceHost`).
pub trait Q3SelectedSourceHost {
    /// Session actors.
    fn actors(&self) -> Rc<dyn Q3SessionActors>;
    /// Shared bodies.
    fn bodies(&self) -> Rc<dyn Q3SessionBodies>;
    /// Gameplay authority.
    fn combat(&self) -> Rc<dyn Q3SessionCombat>;
    /// Shared inventory.
    fn inventory(&self) -> Rc<dyn Q3SessionInventory>;
    /// Actor callbacks.
    fn callbacks(&self) -> Rc<dyn Q3ActorCallbacks>;
    /// World queries, collision, curves, and player curve clip (donor
    /// `Pick<Q3WorldAdapterHost, "queries" | "collision" | "curves" |
    /// "playerCurveClip">`, grounded in the Rust adapter host).
    fn world_queries(&self) -> Rc<dyn Q3WorldAdapterHost>;
    /// Combat provider.
    fn combat_provider(&self) -> ProviderId;
    /// Source damage modifier.
    fn source_damage_modifier(&self) -> Option<SourceDamageModifier>;
    /// Damage powerup owner override.
    fn damage_powerup_owner(&self) -> Option<ProviderId>;
    /// Inventory provider.
    fn inventory_provider(&self) -> ProviderId;
    /// Movement provider.
    fn movement_provider(&self) -> ProviderId;
    /// Victim armor context.
    fn armor_context(&self, request: &DamageRequest) -> VictimArmorContext;
    /// Game type tag.
    fn game_type(&self) -> i32;
    /// Friendly fire.
    fn friendly_fire(&self) -> bool;
    /// Knockback scale.
    fn knockback(&self) -> f32;
    /// Queued intermission.
    fn intermission_queued(&self) -> i32;
    /// Carrier hurt hook.
    fn check_hurt_carrier(&self, target: &ActorId, attacker: &ActorId);
    /// Owning provider.
    fn provider(&self) -> ProviderId;
    /// Product.
    fn product(&self) -> Product;
    /// Equipment ownership.
    fn equipment_kind(&self) -> Q3EquipmentKind;
    /// Weapon effects overrides, if any.
    fn weapon_effects(&self) -> Option<Rc<dyn Q3SourceWeaponEffects>>;
    /// Content identity.
    fn content(&self) -> ContentId;
    /// Configstring store.
    fn configstrings(&self) -> Rc<RefCell<dyn ConfigStringStore>>;
    /// Userinfo string for an actor.
    fn userinfo(&self, actor: &ActorId) -> String;
    /// Configured clients.
    fn max_clients(&self) -> usize;
    /// Random seed.
    fn seed(&self) -> i32;
    /// Current time in milliseconds.
    fn now(&self) -> i32;
    /// World actor.
    fn world_actor(&self) -> OwnedActor;
    /// Client pose for a player actor, if any.
    fn player(&self, actor: &ActorId) -> Option<Q3SelectedClientPose>;
    /// Quad damage factor, optionally for an actor.
    fn quad_factor(&self, actor: Option<&ActorId>) -> f32;
    /// Proximity-mine timeout in milliseconds.
    fn proximity_timeout(&self) -> i32;
    /// Model index for a path.
    fn model_index(&self, path: &str) -> i32;
    /// Sound index for a path.
    fn sound_index(&self, path: &str) -> i32;
    /// Print a line.
    fn print(&self, text: &str);
    /// Called only for component-owned actors; the caller uses the selected
    /// source's frame cadence.
    fn execute(&self, actor: &OwnedActor, step: Box<dyn Fn(i32, i32)>);
    /// Original source events retain their fields and actor identity for
    /// the shared presentation path.
    fn event(&self, actor: &OwnedActor, state: EntityState, time: i32);
    /// Copies of changed source fields; body, health and ammunition
    /// already write their current owners.
    fn client_changed(&self, actor: &OwnedActor, before: &Q3SelectedClientEffects, after: &Q3SelectedClientEffects);
    /// Obelisk attack hook.
    fn check_obelisk_attack(&self, target: &ActorId, attacker: &ActorId) -> bool;
    /// Drop carried objectives.
    fn drop_objectives(&self, actor: &OwnedActor);
    /// Return a pickup to its spawner.
    fn return_pickup(&self, actor: &ActorId);
    /// Spawn point for an actor.
    fn spawn_point(&self, actor: &OwnedActor) -> Q3SpawnPoint;
}

/// One legacy projectile import: the owner-picked slot plus the saved
/// projectile state (donor `restoreLegacyQ3Source` continuations).
#[derive(Debug, Clone, PartialEq)]
pub struct LegacyProjectileImport {
    /// Entity slot picked by the owner.
    pub slot: i32,
    /// Saved projectile state.
    pub state: Q3ProjectileState,
}

/// Legacy import bundle validated by the owner (donor
/// `restoreLegacyQ3Source` inputs).
#[derive(Debug, Clone, PartialEq)]
pub struct LegacyImport {
    /// Projectile imports.
    pub projectiles: Vec<LegacyProjectileImport>,
    /// Weapon statistics.
    pub statistics: Vec<Q3WeaponStatistics>,
    /// Hook-latch actors.
    pub hook_held: Vec<OwnedActor>,
    /// Save time in milliseconds.
    pub milliseconds: i32,
}

/// Game-side save values captured by the backend (donor `capture` graph,
/// missiles, and personal-portal words).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3GameSave {
    /// Entity graph words.
    pub graph: SaveJson,
    /// Missile words.
    pub missiles: SaveJson,
    /// Personal-portal words, if any.
    pub portal: Option<SaveJson>,
}

/// One entity event polled from the backend (donor `publish` event words).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SourceEntityEvent {
    /// Event actor.
    pub actor: OwnedActor,
    /// Copied entity state.
    pub state: EntityState,
    /// Event time in milliseconds.
    pub time: i32,
}

/// Runtime-owned Q3 game backend (V-seam). The donor calls the content
/// game models directly; the Rust game models are self-contained mirrors
/// (see the module docs), so each game-simulation call below crosses into
/// the runtime lane's backend, which runs it over its own composition. The
/// owner keeps records, slots, actors, publishing, and saves.
///
/// The donor threads `host.weaponBehavior` into `MissileRuntime` and shares
/// one `GameRandom` across missiles, weapons, and portals. The behavior
/// port stays runtime-internal (host and backend are both runtime-owned);
/// the random stream is shared explicitly through
/// [`Q3SourceGame::bind_random`] so saves stay faithful.
pub trait Q3SourceGame {
    /// Share the owner's random stream (donor `new MissileRuntime({ ...
    /// random })`, `new WeaponRuntime({ random })`, portal `random`).
    fn bind_random(&self, random: Rc<RefCell<GameRandom>>);
    /// Owning player of a missile actor, if any (donor
    /// `missiles.ownerOf`, wired as the bridge `projectileParent`).
    fn missile_owner_of(&self, actor: &ActorId) -> Option<ActorId>;
    /// Run a think callback: clear next-think, invoke think, fail on NULL
    /// (donor `runThink` closure in the records host).
    fn think(&self, actor: &OwnedActor, time: i32) -> Result<(), Q3SourceError>;
    /// Expire predictable events; returns whether the entity stays active
    /// (donor `pool.expireEvents`, read in `step`).
    fn expire_events(&self, actor: &OwnedActor, time: i32) -> bool;
    /// Run owned missiles; returns whether one consumed the step (donor
    /// `missiles.runOwned`, read in `step`).
    fn run_owned_missiles(&self, actor: &OwnedActor, previous: i32, time: i32) -> bool;
    /// Free a grappling hook; returns whether one was freed (donor
    /// `missiles.hookFree`, guarded by `client.hook != null`).
    fn hook_free(&self, actor: &OwnedActor) -> bool;
    /// Poll entity events raised since the previous call (donor `publish`
    /// event words, read from pool entities the backend owns).
    fn poll_entity_events(&self) -> Vec<Q3SourceEntityEvent>;
    /// Mirror backend entity words into `records` and report newly
    /// activated actors for tracking (donor `publish` pool-entity reads
    /// plus the pool `link` hook, which registers actors for stepping;
    /// records entities have no link lifecycle of their own).
    fn sync_records(&self, records: &Q3EntityRecords) -> Vec<OwnedActor>;
    /// Gauntlet hit check (donor `weapons.checkGauntletAttack`).
    fn check_gauntlet_attack(&self, actor: &OwnedActor) -> bool;
    /// Fire a weapon: predictable event plus weapon behavior (donor
    /// `pool.addPredictableEvent` + `weapons.fire` in `fire`). `None`
    /// asks the backend to compute the base damage factor itself.
    fn fire_weapon(&self, actor: &OwnedActor, weapon: i32, damage_factor: Option<f32>);
    /// Carried persistent-powerup tag, if any (donor
    /// `entity.client.persistantPowerup.item.tag`, read for the damage
    /// factor and the firing delay).
    fn carried_powerup_tag(&self, actor: &OwnedActor) -> Option<i32>;
    /// Collision clip mask (donor `entity.clipmask`, read in `motion`;
    /// the records entity has no clip mask).
    fn clip_mask(&self, actor: &OwnedActor) -> i32;
    /// Project a game entity for a records attachment (donor pool side
    /// of `project`: the pool entity grows with the slot).
    fn project_entity(&self, actor: &OwnedActor, slot: i32, is_player: bool);
    /// Sync persistent client words from a pose (donor `refresh`
    /// `pers.maxHealth` / `pers.netname` writes, which have no records
    /// home).
    fn refresh_client_persistent(&self, actor: &OwnedActor, pose: &Q3SelectedClientPose, netname: &str);
    /// Run a weapon command: clear the fire latch when not attacking or
    /// dead, free the hook when not firing (donor `command`).
    fn command(&self, actor: &OwnedActor, attack: bool, selected: bool, alive: bool);
    /// Accuracy hit hook (donor `logAccuracyHit`, wired as the bridge
    /// `logAccuracyHit`).
    fn log_accuracy_hit(&self, target: &ActorId, attacker: &ActorId) -> bool;
    /// Source damage feedback (Rust-model `Q3CombatBridgeHost` requirement;
    /// the donor bridge has no counterpart).
    fn damage_feedback(&self, target: &ActorId, decision: &DamageDecision);
    /// Foreign damage feedback (Rust-model `Q3CombatBridgeHost`
    /// requirement; the donor bridge has no counterpart).
    fn foreign_damage_feedback(&self, target: &ActorId, owner: Option<&ActorId>, decision: &DamageDecision);
    /// Use a holdable (donor `useQ3Holdable` in `useHoldable`, including
    /// the missionpack portal service). The backend invokes `teleport`
    /// for portal/warp effects.
    fn use_holdable(&self, actor: &OwnedActor, event: i32, teleport: &dyn Fn(&OwnedActor));
    /// Teleport a player (donor `teleportPlayer`).
    fn teleport_player(&self, actor: &OwnedActor, origin: Vec3, angles: Vec3);
    /// Bind a projected pickup entity and run the holdable/persistent
    /// pickup behavior; returns respawn seconds (donor `takePickup` tail:
    /// `pickup.item` assignment plus `pickupHoldable` /
    /// `pickupPersistentPowerup`).
    #[allow(clippy::too_many_arguments)]
    fn take_pickup(
        &self,
        player: &OwnedActor,
        item_actor: &OwnedActor,
        item_index: i32,
        count: i32,
        generic1: i32,
        dropped: bool,
        game_type: i32,
        handicap: &str,
    ) -> i32;
    /// Toss a client's carried persistent powerup, allocating pickup
    /// actors through `spawn` and reporting each tossed pickup through
    /// `on_pickup` (donor `tossQ3ClientPersistentPowerup` in
    /// `returnPersistent`).
    fn toss_client_persistent_powerup(
        &self,
        actor: &OwnedActor,
        spawn: &dyn Fn() -> OwnedActor,
        on_pickup: &dyn Fn(&OwnedActor),
    );
    /// Return an owned persistent pickup to its spawner (donor
    /// `returnQ3PersistentPowerup`).
    fn return_persistent_pickup(&self, pickup: &OwnedActor);
    /// Grant a holdable through a temporary pickup entity (donor
    /// `giveHoldable`: `pool.spawn`, `pickupHoldable`, `pool.free`).
    fn give_holdable(&self, actor: &OwnedActor, item_index: i32);
    /// Refresh powerup timers (donor `updateQ3ClientPowerups` in
    /// `fixedPose` and `publish`).
    fn update_client_powerups(&self, actor: &OwnedActor);
    /// Run client timer actions with the given ammo-timer ownership
    /// (donor `clientTimerActions` in `endCommand`).
    fn client_timer_actions(
        &self,
        actor: &OwnedActor,
        milliseconds: i32,
        ordinary_decay: bool,
        ammo: Option<Vec<Q3MappedAmmoTimer>>,
    );
    /// Expand an invulnerability shell (donor `expandQ3Invulnerability`
    /// in `fixedPose`).
    fn expand_invulnerability(&self, actor: &OwnedActor);
    /// Invulnerability impact effect (donor `invulnerabilityEffect`,
    /// wired as the bridge hook and the missile/missionpack impact).
    fn invulnerability_effect(&self, target: &OwnedActor, direction: Vec3, point: Vec3);
    /// Invulnerability damage block check (donor `q3InvulnerabilityBlocks`
    /// in `blocksDamage`).
    fn invulnerability_blocks(&self, target: &ActorId, direction: Vec3, point: Vec3, means_of_death: i32) -> bool;
    /// Client speed multiplier (donor `clientSpeedMultiplier` in
    /// `speedMultiplier`; needs the team-arena item host).
    fn speed_multiplier(&self, actor: &ActorId) -> f32;
    /// Capture the game-side save words (donor `captureQ3Graph`,
    /// `missiles.captureSaveState`, `personalPortal.captureSaveState`).
    fn capture_game(&self) -> Q3GameSave;
    /// Restore the entity graph words and repopulate `records`
    /// (donor `prepareQ3Graph` + `restoreQ3Graph`).
    fn restore_graph(
        &self,
        records: &Q3EntityRecords,
        graph: &SaveJson,
        resolve: &dyn Fn(SavedActorId) -> Option<OwnedActor>,
        reference: &dyn Fn(SavedActorId) -> ActorId,
    ) -> Result<(), Q3SourceError>;
    /// Restore personal-portal words (donor
    /// `personalPortal.restoreSaveState`).
    fn restore_portal(&self, value: &SaveJson) -> Result<(), Q3SourceError>;
    /// Restore missile words with saved-actor references (donor
    /// `missiles.restoreSaveState` in `restore`).
    fn restore_missiles(
        &self,
        value: &SaveJson,
        reference: &dyn Fn(SavedActorId) -> ActorId,
    ) -> Result<(), Q3SourceError>;
    /// Close the missile runtime (donor `missiles.close`).
    fn close_missiles(&self);
    /// Free entity slots available for legacy imports (donor
    /// `pool.at(slot).inuse` scan in `restoreLegacyQ3Source`).
    fn free_entity_slots(&self) -> Vec<i32>;
    /// Import validated legacy projectiles, statistics, hook latches, and
    /// the random seed (donor `restoreLegacyQ3Source` writes).
    fn import_legacy(&self, import: &LegacyImport) -> Result<(), Q3SourceError>;
    /// Client presentation configstring (donor `clientPresentationConfig`
    /// in `refresh`; needs the team-arena client).
    fn client_presentation_config(&self, actor: &OwnedActor, userinfo: &str, game_type: i32) -> String;
    /// Pool presentation state (donor `q3PoolPresentationState` in
    /// `sourceState`; owned by the presentation lane).
    fn pool_presentation_state(&self) -> Q3SourcePresentationState;
    /// Pool models (donor `q3PoolModels` in `presentations`; owned by the
    /// presentation lane).
    fn pool_models(&self) -> Vec<SimulationPresentation>;
}

fn client_effects(ps: &PlayerState, invulnerability_time: i32, max_health: i32) -> Q3SelectedClientEffects {
    Q3SelectedClientEffects {
        teleport_bit: ps.e_flags & TELEPORT_BIT,
        view_angles: ps.viewangles,
        delta_angles: ps.delta_angles,
        pm_flags: ps.pm_flags,
        pm_time: ps.pm_time,
        invulnerability_time,
        max_health,
    }
}

fn same_effects(a: &Q3SelectedClientEffects, b: &Q3SelectedClientEffects) -> bool {
    a.teleport_bit == b.teleport_bit
        && a.pm_flags == b.pm_flags
        && a.pm_time == b.pm_time
        && a.invulnerability_time == b.invulnerability_time
        && a.max_health == b.max_health
        && a.view_angles == b.view_angles
        && a.delta_angles == b.delta_angles
}

const WORLD_SLOT: usize = 1022;
const CONFIG_CLIENT_BASE: usize = 544;
const TELEPORT_BIT: i32 = 4;
const TRIGGER_CONTENTS: i32 = 0x40000000;

/// Convert a content checkpoint value into a world checkpoint value.
/// The two `SaveJson` shapes are identical; no crate converts between
/// them yet.
fn content_save_to_world(value: &qa_content::value::SaveJson) -> SaveJson {
    use qa_content::value::SaveJson as ContentSave;
    match value {
        ContentSave::Null => SaveJson::Null,
        ContentSave::Bool(value) => SaveJson::Bool(*value),
        ContentSave::Number(value) => SaveJson::Number(*value),
        ContentSave::BigInt(value) => SaveJson::BigInt(*value),
        ContentSave::Bytes(value) => SaveJson::Bytes(value.clone()),
        ContentSave::String(value) => SaveJson::String(value.clone()),
        ContentSave::Array(values) => SaveJson::Array(values.iter().map(content_save_to_world).collect()),
        ContentSave::Object(members) => SaveJson::Object(
            members
                .iter()
                .map(|(key, value)| (key.clone(), content_save_to_world(value)))
                .collect(),
        ),
    }
}

/// Convert a world checkpoint value into a content checkpoint value.
fn world_save_to_content(value: &SaveJson) -> qa_content::value::SaveJson {
    use qa_content::value::SaveJson as ContentSave;
    match value {
        SaveJson::Null => ContentSave::Null,
        SaveJson::Bool(value) => ContentSave::Bool(*value),
        SaveJson::Number(value) => ContentSave::Number(*value),
        SaveJson::BigInt(value) => ContentSave::BigInt(*value),
        SaveJson::Bytes(value) => ContentSave::Bytes(value.clone()),
        SaveJson::String(value) => ContentSave::String(value.clone()),
        SaveJson::Array(values) => ContentSave::Array(values.iter().map(world_save_to_content).collect()),
        SaveJson::Object(members) => ContentSave::Object(
            members
                .iter()
                .map(|(key, value)| (key.clone(), world_save_to_content(value)))
                .collect(),
        ),
    }
}

fn stat_max_health(product: Product) -> usize {
    use qa_content::q3::base::shared::definitions::StatSchema;
    match qa_content::q3::base::shared::definitions::stat_schema(product) {
        StatSchema::Base(layout) => layout.max_health as usize,
        StatSchema::Missionpack(layout) => layout.max_health as usize,
    }
}

struct RecordsPoolView {
    records: Q3EntityRecords,
}

impl Q3EntityPool for RecordsPoolView {
    fn num_entities(&self) -> usize {
        self.records.capture_ownership().len()
    }

    fn entity_at(&self, index: usize) -> EntityRef {
        self.records
            .get(index)
            .unwrap_or_else(|| panic!("Q3 entity {index} outside 0..1024"))
    }
}

/// Published event words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PublishedEvent {
    event: i32,
    time: i32,
}

/// Records host wiring the session tables to the owner core (donor
/// `Q3EntityRecords` options in the constructor).
struct SourceRecordHost {
    host: Rc<dyn Q3SelectedSourceHost>,
    core: Weak<Q3SelectedSourceCore>,
}

impl SourceRecordHost {
    fn core(&self) -> Rc<Q3SelectedSourceCore> {
        self.core.upgrade().expect("Selected Q3 source is closed")
    }
}

impl Q3RecordHost for SourceRecordHost {
    fn actors(&self) -> Rc<dyn Q3SessionActors> {
        self.host.actors()
    }

    fn bodies(&self) -> Rc<dyn Q3SessionBodies> {
        self.host.bodies()
    }

    fn combat(&self) -> Rc<dyn Q3SessionCombat> {
        self.host.combat()
    }

    fn inventory(&self) -> Rc<dyn Q3SessionInventory> {
        self.host.inventory()
    }

    fn callbacks(&self) -> Rc<dyn Q3ActorCallbacks> {
        self.host.callbacks()
    }

    fn schedule(&self, actor: &OwnedActor, _due_milliseconds: Option<i32>) {
        self.core().track(actor);
    }

    fn run_think(&self, actor: &OwnedActor, time_milliseconds: i32) {
        let core = self.core();
        let native = core.records.native_by_actor(Some(actor.id()));
        if native.is_none() || !core.owns(actor) {
            panic!("Selected Q3 think actor is retired");
        }
        if let Err(error) = core.game.think(actor, time_milliseconds) {
            panic!("{error}");
        }
    }

    fn damage_call(&self) -> Option<Q3DamageCall> {
        self.core().bridge.current_call()
    }

    fn foreign(&self, actor: &ActorId) -> Option<EntityRef> {
        match self.core().project(actor) {
            Ok(entity) => entity,
            Err(error) => panic!("{error}"),
        }
    }

    fn is_player(&self, actor: &ActorId) -> bool {
        self.host.player(actor).is_some()
    }
}

/// World host delegating to the host queries, gating collision stores on
/// ownership (donor `Q3WorldAdapter` options `collision` gate).
struct SourceWorldHost {
    inner: Rc<dyn Q3WorldAdapterHost>,
    core: Weak<Q3SelectedSourceCore>,
}

impl Q3WorldAdapterHost for SourceWorldHost {
    fn trace_scene(&self, query: &Q3TraceQuery) -> Q3TraceResult {
        self.inner.trace_scene(query)
    }

    fn point_contents_scene(&self, query: &Q3TraceQuery, point: Vec3) -> i32 {
        self.inner.point_contents_scene(query, point)
    }

    fn query_actors(&self, bounds: Bounds) -> Vec<ActorId> {
        self.inner.query_actors(bounds)
    }

    fn spatial_collision(&self, actor: &ActorId) -> Option<ActorCollision> {
        self.inner.spatial_collision(actor)
    }

    fn body_state(&self, actor: &ActorId) -> Option<BodyState> {
        self.inner.body_state(actor)
    }

    fn linked_body(&self, actor: &ActorId) -> Option<qa_world::body::LinkedBody> {
        self.inner.linked_body(actor)
    }

    fn set_collision(&self, actor: &OwnedActor, collision: ActorCollision) {
        if self.core.upgrade().is_some_and(|core| core.owns(actor)) {
            self.inner.set_collision(actor, collision);
        }
    }

    fn link_body(&self, actor: &OwnedActor, origin: Option<Vec3>) {
        self.inner.link_body(actor, origin);
    }

    fn unlink_body(&self, actor: &OwnedActor) {
        self.inner.unlink_body(actor);
    }

    fn curves(&self) -> bool {
        self.inner.curves()
    }

    fn player_curve_clip(&self) -> bool {
        self.inner.player_curve_clip()
    }

    fn geometry_trace_start_solid(&self, query: &Q3TraceQuery, model: i32, origin: Vec3, angles: Vec3) -> bool {
        self.inner.geometry_trace_start_solid(query, model, origin, angles)
    }

    fn body_trace_start_solid(&self, query: &Q3TraceQuery, body: &BodyState, collision: &ActorCollision) -> bool {
        self.inner.body_trace_start_solid(query, body, collision)
    }
}

/// Combat-bridge host over the owner core (donor `Q3CombatBridge` options
/// in the constructor).
struct SourceBridgeHost {
    host: Rc<dyn Q3SelectedSourceHost>,
    game: Rc<dyn Q3SourceGame>,
    core: Weak<Q3SelectedSourceCore>,
    records: Q3EntityRecords,
    world: Q3WorldAdapter,
}

impl SourceBridgeHost {
    fn actor_of(&self, entity: &EntityRef) -> Option<OwnedActor> {
        let slot = entity.borrow().slot;
        self.records
            .capture_ownership()
            .get(slot)
            .and_then(|record| record.actor.clone())
    }
}

impl Q3CombatBridgeHost for SourceBridgeHost {
    fn authority(&self) -> Rc<dyn Q3SessionCombat> {
        self.host.combat()
    }

    fn entities(&self) -> Rc<dyn Q3EntityPool> {
        Rc::new(RecordsPoolView {
            records: self.records.clone(),
        })
    }

    fn records(&self) -> Q3EntityRecords {
        self.records.clone()
    }

    fn world(&self) -> Rc<dyn qa_content::q3::base::world::Q3ServerWorld> {
        Rc::new(self.world.clone())
    }

    fn weapon_provider(&self) -> ProviderId {
        self.host.provider()
    }

    fn damage_powerup_owner(&self) -> Option<ProviderId> {
        self.host.damage_powerup_owner()
    }

    fn source_damage_modifier(&self) -> Option<SourceDamageModifier> {
        self.host.source_damage_modifier()
    }

    fn combat_provider(&self) -> ProviderId {
        self.host.combat_provider()
    }

    fn inventory_provider(&self) -> ProviderId {
        self.host.inventory_provider()
    }

    fn movement_provider(&self) -> ProviderId {
        self.host.movement_provider()
    }

    fn armor_context(&self, request: &DamageRequest) -> VictimArmorContext {
        self.host.armor_context(request)
    }

    fn time(&self) -> i32 {
        self.core.upgrade().map(|core| core.now()).unwrap_or(0)
    }

    fn intermission_queued(&self) -> i32 {
        self.host.intermission_queued()
    }

    fn game_type(&self) -> i32 {
        self.host.game_type()
    }

    fn friendly_fire(&self) -> bool {
        self.host.friendly_fire()
    }

    fn knockback(&self) -> f32 {
        self.host.knockback()
    }

    fn product(&self) -> Product {
        self.host.product()
    }

    fn check_hurt_carrier(&self, target: EntityRef, attacker: EntityRef) {
        if let (Some(target), Some(attacker)) = (self.actor_of(&target), self.actor_of(&attacker)) {
            self.host.check_hurt_carrier(target.id(), attacker.id());
        }
    }

    fn log_accuracy_hit(&self, target: EntityRef, attacker: EntityRef) -> bool {
        match (self.actor_of(&target), self.actor_of(&attacker)) {
            (Some(target), Some(attacker)) => self.game.log_accuracy_hit(target.id(), attacker.id()),
            _ => false,
        }
    }

    fn damage_feedback(&self, call: &Q3DamageCall, decision: &DamageDecision) {
        if let Some(target) = self.actor_of(&call.target) {
            self.game.damage_feedback(target.id(), decision);
        }
    }

    fn foreign_damage_feedback(&self, target: EntityRef, owner: Option<EntityRef>, decision: &DamageDecision) {
        let target = self.actor_of(&target);
        let owner = owner.as_ref().and_then(|owner| self.actor_of(owner));
        if let Some(target) = target {
            self.game
                .foreign_damage_feedback(target.id(), owner.as_ref().map(|owner| owner.id()), decision);
        }
    }

    fn projectile_parent(&self, actor: &ActorId) -> Option<ActorId> {
        self.game.missile_owner_of(actor)
    }

    fn check_obelisk_attack(&self, target: EntityRef, attacker: EntityRef) -> bool {
        match (self.actor_of(&target), self.actor_of(&attacker)) {
            (Some(target), Some(attacker)) => self.host.check_obelisk_attack(target.id(), attacker.id()),
            _ => false,
        }
    }

    fn invulnerability_effect(&self, target: EntityRef, direction: Vec3, point: Vec3) {
        if let Some(target) = self.actor_of(&target) {
            self.game.invulnerability_effect(&target, direction, point);
        }
    }
}

/// Owner core: records, world, bridge, shared random, and the projection,
/// execution, publication, and client-effect tables. All interior
/// mutability; methods never hold a borrow across host or backend calls
/// so the host-driven `execute`/`step` reentrancy cannot trap.
struct Q3SelectedSourceCore {
    host: Rc<dyn Q3SelectedSourceHost>,
    game: Rc<dyn Q3SourceGame>,
    records: Q3EntityRecords,
    world: Q3WorldAdapter,
    bridge: Q3CombatBridge,
    random: Rc<RefCell<GameRandom>>,
    projected: RefCell<HashMap<ActorId, (OwnedActor, usize)>>,
    client_effects: RefCell<HashMap<ActorId, Q3SelectedClientEffects>>,
    executing: RefCell<HashSet<ActorId>>,
    published: RefCell<HashMap<ActorId, PublishedEvent>>,
    unobserve: RefCell<Option<Box<dyn Fn()>>>,
    depth: Cell<i32>,
    previous: Cell<i32>,
    clock: Cell<Option<i32>>,
    closed: Cell<bool>,
    revision: Cell<i32>,
    restored_presentation: RefCell<Option<Q3SourcePresentationState>>,
    event_times: RefCell<HashMap<ActorId, i32>>,
    own: Weak<Q3SelectedSourceCore>,
}

impl Q3SelectedSourceCore {
    fn now(&self) -> i32 {
        self.clock.get().unwrap_or_else(|| self.host.now())
    }

    fn assert_open(&self) -> Result<(), Q3SourceError> {
        if self.closed.get() {
            return Err(Q3SourceError::Closed);
        }
        Ok(())
    }

    fn owns(&self, actor: &OwnedActor) -> bool {
        actor.owner() == &self.host.provider() && self.records.native_by_actor(Some(actor.id())).is_some()
    }

    fn actor_of_slot(&self, slot: usize) -> Option<OwnedActor> {
        self.records
            .capture_ownership()
            .get(slot)
            .and_then(|record| record.actor.clone())
    }

    fn slot_active(&self, slot: usize) -> bool {
        self.records
            .capture_ownership()
            .get(slot)
            .is_some_and(|record| record.active)
    }

    fn track(&self, actor: &OwnedActor) {
        if !self.owns(actor) || self.executing.borrow().contains(actor.id()) {
            return;
        }
        self.executing.borrow_mut().insert(actor.id().clone());
        let owned = actor.clone();
        let core = self.own.clone();
        self.host.execute(
            actor,
            Box::new(move |previous, time| {
                if let Some(core) = core.upgrade() {
                    core.step(&owned, previous, time);
                }
            }),
        );
    }

    fn project(&self, actor: &ActorId) -> Result<Option<EntityRef>, Q3SourceError> {
        let Some(owned) = self.host.actors().resolve_owned(actor) else {
            return Ok(None);
        };
        if let Some(current) = self.records.native_by_actor(Some(actor)) {
            return Ok(Some(current));
        }
        if self.host.bodies().read(actor).is_none() {
            return Err(Q3SourceError::MissingBody);
        }
        let pose = self.host.player(actor);
        let is_player = pose.is_some();
        let (mut slot, end) = if is_player {
            (0usize, self.host.max_clients().min(MAX_CLIENTS))
        } else {
            (MAX_CLIENTS, WORLD_SLOT)
        };
        while slot < end && self.slot_active(slot) {
            slot += 1;
        }
        if slot >= end {
            return Err(Q3SourceError::ProjectionCapacity);
        }
        let entity = self.records.attach(slot, owned.clone(), is_player)?;
        self.projected.borrow_mut().insert(actor.clone(), (owned.clone(), slot));
        self.game.project_entity(&owned, slot as i32, is_player);
        if is_player {
            let client = self.records.client(slot);
            let fresh = GameClient::new(self.host.product(), None, None);
            {
                let mut guard = client.borrow_mut();
                guard.ps.copy_from(&fresh.ps, AuthorityStores::PreserveAuthority);
                guard.noclip = fresh.noclip;
                guard.invulnerability_time = fresh.invulnerability_time;
                guard.ammo_times = fresh.ammo_times;
                guard.sess = fresh.sess;
            }
            self.records.restore_client_backing(
                slot,
                &ClientBackingSnapshot {
                    source_stats: [0; 16],
                    special_ammo: [0; 16],
                },
            )?;
            entity.borrow_mut().s.e_type = EntityType::EtPlayer as i32;
            entity.borrow_mut().s.client_num = slot as i32;
            client.borrow_mut().ps.client_num = slot as i32;
        }
        self.refresh(&entity)?;
        Ok(Some(entity))
    }

    fn refresh(&self, entity: &EntityRef) -> Result<(), Q3SourceError> {
        let (actor, slot) = {
            let guard = entity.borrow();
            let Some(actor) = self.actor_of_slot(guard.slot) else {
                return Err(Q3SourceError::MissingBody);
            };
            (actor, guard.slot)
        };
        let Some(body) = self.host.bodies().read(actor.id()) else {
            return Err(Q3SourceError::MissingBody);
        };
        {
            let mut guard = entity.borrow_mut();
            guard.s.pos = Trajectory {
                trajectory_type: TrajectoryType::TrStationary,
                time: 0,
                duration: 0,
                base: body.origin,
                delta: Vec3::default(),
            };
            guard.s.apos = Trajectory {
                trajectory_type: TrajectoryType::TrStationary,
                time: 0,
                duration: 0,
                base: body.angles,
                delta: Vec3::default(),
            };
        }
        let client = entity.borrow().client.clone();
        let pose = self.host.player(actor.id());
        let (Some(client), Some(pose)) = (client, pose) else {
            return Ok(());
        };
        self.publish_client(&actor, &client);
        {
            let mut guard = client.borrow_mut();
            guard.ps.viewangles = pose.angles;
            guard.ps.viewheight = pose.view_height;
            guard.sess.session_team = pose.team;
            guard
                .ps
                .persistant
                .set(PersistentIndex::PersTeam as usize, pose.team as i32);
            let max_health = stat_max_health(self.host.product());
            guard.ps.stats.set(max_health, pose.max_health);
            guard.ps.powerups.set(Powerup::PwQuad as usize, pose.quad_until);
            guard.ps.powerups.set(Powerup::PwHaste as usize, pose.haste_until);
        }
        let userinfo = self.host.userinfo(actor.id());
        let netname = clean_client_name(&client_info_value(&userinfo, "name"));
        self.game.refresh_client_persistent(&actor, &pose, &netname);
        let config = self
            .game
            .client_presentation_config(&actor, &userinfo, self.host.game_type());
        self.host
            .configstrings()
            .borrow_mut()
            .set(CONFIG_CLIENT_BASE + slot, &config);
        let effects = self.effects_of(&client);
        self.client_effects.borrow_mut().insert(actor.id().clone(), effects);
        Ok(())
    }

    fn effects_of(&self, client: &ClientRef) -> Q3SelectedClientEffects {
        let ps = client.borrow().ps.clone();
        let invulnerability_time = client.borrow().invulnerability_time;
        let max_health = ps.stats.get(stat_max_health(self.host.product()));
        client_effects(&ps, invulnerability_time, max_health)
    }

    fn publish_client(&self, actor: &OwnedActor, client: &ClientRef) {
        let after = self.effects_of(client);
        let before = self
            .client_effects
            .borrow_mut()
            .insert(actor.id().clone(), after.clone());
        if before.as_ref().is_some_and(|before| !same_effects(before, &after)) {
            let before = before.unwrap_or(after.clone());
            self.host.client_changed(actor, &before, &after);
        }
    }

    fn bind_world(&self) -> Result<(), Q3SourceError> {
        let world = self.host.world_actor();
        match self.records.native_by_actor(Some(world.id())) {
            None => {
                self.records.attach(WORLD_SLOT, world, false)?;
            }
            Some(current) => {
                if current.borrow().slot != WORLD_SLOT {
                    return Err(Q3SourceError::WorldSlotChanged);
                }
            }
        }
        Ok(())
    }

    fn player(&self, actor: &OwnedActor) -> Result<EntityRef, Q3SourceError> {
        self.assert_open()?;
        self.host.actors().assert_owned(actor)?;
        self.bind_world()?;
        let entity = self.project(actor.id())?;
        let has_client = entity.as_ref().is_some_and(|entity| entity.borrow().client.is_some());
        if !has_client {
            return Err(Q3SourceError::MissingPlayer);
        }
        let entity = entity.unwrap_or_else(|| panic!("Selected Q3 arsenal requires an admitted player"));
        self.refresh(&entity)?;
        Ok(entity)
    }

    fn step(&self, actor: &OwnedActor, previous: i32, time: i32) {
        if self.closed.get() || !self.host.actors().is_live(actor.id()) || !self.owns(actor) {
            return;
        }
        let prior = self.clock.get();
        let prior_previous = self.previous.get();
        self.clock.set(Some(time));
        self.previous.set(previous);
        let projected: Vec<(OwnedActor, usize)> = self.projected.borrow().values().cloned().collect();
        for (borrowed, slot) in &projected {
            if self.host.actors().is_live(borrowed.id()) {
                if let Some(entity) = self.records.get(*slot) {
                    if let Err(error) = self.refresh(&entity) {
                        panic!("{error}");
                    }
                }
            }
        }
        let entity = self.records.native_by_actor(Some(actor.id()));
        if let Some(entity) = entity {
            if let Err(error) = self.run(|core| {
                let active = core.game.expire_events(actor, time);
                if !active {
                    entity.borrow_mut().s.event = 0;
                }
                if active && !core.game.run_owned_missiles(actor, previous, time) {
                    core.game.think(actor, time)?;
                }
                Ok(())
            }) {
                panic!("{error}");
            }
        }
        self.clock.set(prior);
        self.previous.set(prior_previous);
    }

    fn run<T>(
        &self,
        operation: impl FnOnce(&Q3SelectedSourceCore) -> Result<T, Q3SourceError>,
    ) -> Result<T, Q3SourceError> {
        self.assert_open()?;
        self.depth.set(self.depth.get() + 1);
        let result = operation(self);
        self.publish();
        self.depth.set(self.depth.get() - 1);
        result
    }

    /// Publish shared callbacks and source events. The records entity
    /// has no `eventTime` word (donor pool entities do), so event times
    /// live in `event_times`, keyed by actor.
    fn publish(&self) {
        for actor in self.game.sync_records(&self.records) {
            self.track(&actor);
        }
        for event in self.game.poll_entity_events() {
            let Some(entity) = self.records.native_by_actor(Some(event.actor.id())) else {
                continue;
            };
            let is_client = entity.borrow().client.is_some();
            if is_client {
                let mut guard = entity.borrow_mut();
                guard.s.event = event.state.event;
                guard.s.event_parm = event.state.event_parm;
            } else {
                entity.borrow_mut().s = event.state.clone();
            }
            self.event_times
                .borrow_mut()
                .insert(event.actor.id().clone(), event.time);
        }
        let ownership = self.records.capture_ownership();
        for (slot, record) in ownership.iter().enumerate() {
            if !record.active {
                continue;
            }
            let Some(entity) = self.records.get(slot) else {
                continue;
            };
            let client = entity.borrow().client.clone();
            if let Some(client) = client {
                let Some(actor) = record.actor.clone() else {
                    continue;
                };
                self.publish_client(&actor, &client);
                self.game.update_client_powerups(&actor);
                // Clone first: player-state stat reads re-borrow the entity
                // through the records stat binding.
                let mut ps = client.borrow().ps.clone();
                let mut s = entity.borrow().s.clone();
                player_state_to_entity_state(&mut ps, &mut s, true);
                client.borrow_mut().ps = ps;
                entity.borrow_mut().s = s;
            } else {
                let Some(actor) = record.actor.clone() else {
                    continue;
                };
                if !self.owns(&actor) {
                    continue;
                }
                self.track(&actor);
            }
        }
        for (slot, record) in ownership.iter().enumerate() {
            if !record.active {
                continue;
            }
            let Some(actor) = record.actor.clone() else {
                continue;
            };
            let Some(entity) = self.records.get(slot) else {
                continue;
            };
            let event = {
                let guard = entity.borrow();
                if guard.s.e_type >= EntityType::EtEvents as i32 {
                    guard.s.e_type - EntityType::EtEvents as i32
                } else {
                    guard.s.event
                }
            };
            if event == 0 {
                continue;
            }
            let event_time = self.event_times.borrow().get(actor.id()).copied().unwrap_or(0);
            let previous = self.published.borrow().get(actor.id()).copied();
            if previous.is_some_and(|previous| previous.event == event && previous.time == event_time) {
                continue;
            }
            self.published.borrow_mut().insert(
                actor.id().clone(),
                PublishedEvent {
                    event,
                    time: event_time,
                },
            );
            let state = self.records.get(slot).map(|entity| entity.borrow().s.clone());
            if let Some(state) = state {
                self.host.event(&actor, state, event_time);
            }
        }
    }
}

/// Original Q3 weapon/equipment records over actors admitted by their
/// actual world (donor `Q3SelectedSource`).
pub struct Q3SelectedSource {
    core: Rc<Q3SelectedSourceCore>,
}

impl Q3SelectedSource {
    /// Create a selected Q3 source over a host and a game backend. The
    /// donor constructor's `restored` value unfolds to an explicit
    /// [`Q3SelectedSource::restore`] call, which needs the runtime's
    /// saved-actor resolvers.
    pub fn new(host: Rc<dyn Q3SelectedSourceHost>, game: Rc<dyn Q3SourceGame>) -> Self {
        let seed = host.seed();
        let core = Rc::new_cyclic(|own| {
            let record_host = Rc::new(SourceRecordHost {
                host: host.clone(),
                core: own.clone(),
            });
            let records = Q3EntityRecords::new(record_host, host.provider(), host.product());
            let world_host = Rc::new(SourceWorldHost {
                inner: host.world_queries(),
                core: own.clone(),
            });
            let world = Q3WorldAdapter::new(world_host, records.clone());
            let bridge = Q3CombatBridge::new(Rc::new(SourceBridgeHost {
                host: host.clone(),
                game: game.clone(),
                core: own.clone(),
                records: records.clone(),
                world: world.clone(),
            }));
            let unobserve_host = host.clone();
            let unobserve_game = game.clone();
            let unobserve_records = records.clone();
            let unobserve = host.actors().on_release(Box::new({
                let own = own.clone();
                move |actor: &OwnedActor| {
                    let Some(core) = own.upgrade() else {
                        return;
                    };
                    core.executing.borrow_mut().remove(actor.id());
                    core.published.borrow_mut().remove(actor.id());
                    core.client_effects.borrow_mut().remove(actor.id());
                    core.event_times.borrow_mut().remove(actor.id());
                    let slot = core.projected.borrow_mut().remove(actor.id()).map(|(_, slot)| slot);
                    if let Some(slot) = slot {
                        if slot < unobserve_host.max_clients() {
                            let _ = &unobserve_game;
                            if let Some(actor) = core.actor_of_slot(slot) {
                                core.return_persistent(&actor);
                            }
                            unobserve_host
                                .configstrings()
                                .borrow_mut()
                                .set(CONFIG_CLIENT_BASE + slot, "");
                        }
                        if let Some(entity) = unobserve_records.get(slot) {
                            unobserve_records.release(entity);
                        }
                    }
                }
            }));
            Q3SelectedSourceCore {
                host,
                game,
                records,
                world,
                bridge,
                random: Rc::new(RefCell::new(GameRandom::new(seed))),
                projected: RefCell::new(HashMap::new()),
                client_effects: RefCell::new(HashMap::new()),
                executing: RefCell::new(HashSet::new()),
                published: RefCell::new(HashMap::new()),
                unobserve: RefCell::new(Some(unobserve)),
                depth: Cell::new(0),
                previous: Cell::new(0),
                clock: Cell::new(None),
                closed: Cell::new(false),
                revision: Cell::new(0),
                restored_presentation: RefCell::new(None),
                event_times: RefCell::new(HashMap::new()),
                own: own.clone(),
            }
        });
        core.game.bind_random(core.random.clone());
        Q3SelectedSource { core }
    }

    /// Entity records.
    pub fn records(&self) -> Q3EntityRecords {
        self.core.records.clone()
    }

    /// Server world adapter.
    pub fn world(&self) -> Q3WorldAdapter {
        self.core.world.clone()
    }

    /// Combat bridge.
    pub fn bridge(&self) -> Q3CombatBridge {
        self.core.bridge.clone()
    }

    /// Game backend (donor `missiles`, `weapons`, `personalPortal`).
    pub fn game(&self) -> Rc<dyn Q3SourceGame> {
        self.core.game.clone()
    }

    /// Shared random stream.
    pub fn random(&self) -> Rc<RefCell<GameRandom>> {
        self.core.random.clone()
    }

    /// Source host.
    pub fn host(&self) -> Rc<dyn Q3SelectedSourceHost> {
        self.core.host.clone()
    }

    /// Generation counter, bumped on every restore.
    pub fn generation(&self) -> i32 {
        self.core.revision.get()
    }

    /// Whether the source owns equipment.
    pub fn owns_equipment(&self) -> bool {
        self.core.host.equipment_kind() == Q3EquipmentKind::Source
    }

    /// Presentation baseline captured by the latest restore, if any.
    pub fn presentation_baseline(&self) -> Option<Q3SourcePresentationState> {
        self.core.restored_presentation.borrow().clone()
    }

    /// Whether the source is open.
    pub fn active(&self) -> bool {
        !self.core.closed.get()
    }

    /// Actor at a source slot, if live.
    pub fn actor(&self, slot: usize) -> Option<ActorId> {
        if self.core.closed.get() {
            return None;
        }
        let record = self.core.records.capture_ownership().get(slot)?.clone();
        if !record.active {
            return None;
        }
        let actor = record.actor?;
        if !self.core.host.actors().is_live(actor.id()) {
            return None;
        }
        Some(actor.id().clone())
    }

    /// Whether an actor has a live native entity.
    pub fn live(&self, actor: &ActorId) -> bool {
        !self.core.closed.get()
            && self.core.host.actors().is_live(actor)
            && self.core.records.native_by_actor(Some(actor)).is_some()
    }
}

/// Records-backed player inventory for the grab checks (donor
/// `q3ItemInventory(client)`).
struct SourcePlayerInventory {
    product: Product,
    health: i32,
    armor: i32,
    max_health: i32,
    holdable_item: i32,
    team: i32,
    ammo: Vec<i32>,
    powerups: Vec<i32>,
    persistent_powerup_index: i32,
}

impl SharedPlayerInventory for SourcePlayerInventory {
    fn product(&self) -> Product {
        self.product
    }

    fn health(&self) -> i32 {
        self.health
    }

    fn armor(&self) -> i32 {
        self.armor
    }

    fn max_health(&self) -> i32 {
        self.max_health
    }

    fn holdable_item(&self) -> i32 {
        self.holdable_item
    }

    fn team(&self) -> i32 {
        self.team
    }

    fn ammo(&self, weapon: qa_content::q3::base::shared::definitions::Weapon) -> i32 {
        self.ammo.get(weapon as usize).copied().unwrap_or(0)
    }

    fn powerup(&self, powerup: Powerup) -> i32 {
        self.powerups.get(powerup as usize).copied().unwrap_or(0)
    }

    fn persistent_powerup_index(&self) -> i32 {
        self.persistent_powerup_index
    }
}

impl Q3SelectedSourceCore {
    fn player_inventory(&self, actor: &ActorId) -> Option<SourcePlayerInventory> {
        let entity = self.records.native_by_actor(Some(actor))?;
        let client = entity.borrow().client.clone()?;
        let ps = client.borrow().ps.clone();
        let sess_team = client.borrow().sess.session_team;
        let (armor, holdable_item, persistent_powerup_index) = match self.host.product() {
            Product::Baseq3 => {
                let layout = match qa_content::q3::base::shared::definitions::stat_schema(Product::Baseq3) {
                    StatSchema::Base(layout) => layout,
                    StatSchema::Missionpack(_) => panic!("stat schema product mismatch"),
                };
                (
                    ps.stats.get(layout.armor as usize),
                    ps.stats.get(layout.holdable_item as usize),
                    0,
                )
            }
            Product::Missionpack => {
                let layout = match qa_content::q3::base::shared::definitions::stat_schema(Product::Missionpack) {
                    StatSchema::Missionpack(layout) => layout,
                    StatSchema::Base(_) => panic!("stat schema product mismatch"),
                };
                (
                    ps.stats.get(layout.armor as usize),
                    ps.stats.get(layout.holdable_item as usize),
                    ps.stats.get(layout.persistent_powerup as usize),
                )
            }
        };
        let ammo = (0..ps.ammo.len()).map(|index| ps.ammo.get(index)).collect();
        let powerups = (0..ps.powerups.len()).map(|index| ps.powerups.get(index)).collect();
        Some(SourcePlayerInventory {
            product: self.host.product(),
            health: self.host.combat().read(actor).map(|combat| combat.health).unwrap_or(0),
            armor,
            max_health: ps.stats.get(stat_max_health(self.host.product())),
            holdable_item,
            team: sess_team as i32,
            ammo,
            powerups,
            persistent_powerup_index,
        })
    }

    fn owner_of(&self, entity: &EntityRef) -> Option<ActorId> {
        let owner_num = entity.borrow().r.owner_num;
        if owner_num < 0 {
            return None;
        }
        let record = self.records.capture_ownership().get(owner_num as usize)?.clone();
        if !record.active {
            return None;
        }
        record.actor.map(|actor| actor.id().clone())
    }

    fn return_persistent(&self, actor: &OwnedActor) {
        let host = self.host.clone();
        let game = self.game.clone();
        let this = self.own.clone();
        game.toss_client_persistent_powerup(
            actor,
            &|| {
                let core = this.upgrade().expect("Selected Q3 source is closed");
                let ownership = core.records.capture_ownership();
                let slot =
                    (MAX_CLIENTS..WORLD_SLOT).find(|slot| ownership.get(*slot).is_none_or(|record| !record.active));
                let Some(slot) = slot else {
                    panic!("Selected Q3 source projection capacity exceeded");
                };
                let _ = core.records.activate(slot);
                core.actor_of_slot(slot).expect("activated slot has an actor")
            },
            &|pickup| {
                let core = this.upgrade().expect("Selected Q3 source is closed");
                if core.owns(pickup) {
                    game.return_persistent_pickup(pickup);
                } else {
                    host.return_pickup(pickup.id());
                }
            },
        );
    }
}

impl Q3SelectedSource {
    /// Admit a player actor, projecting and refreshing its records.
    pub fn admit(&self, actor: &OwnedActor) -> Result<(), Q3SourceError> {
        self.core.player(actor)?;
        Ok(())
    }

    /// Stationary motion record for an actor.
    pub fn motion(&self, actor: &OwnedActor, body: &BodyState) -> Result<Q2Motion, Q3SourceError> {
        let Some(entity) = self.core.records.native_by_actor(Some(actor.id())) else {
            return Err(Q3SourceError::RetiredMotion);
        };
        if !self.core.owns(actor) {
            return Err(Q3SourceError::RetiredMotion);
        }
        Ok(Q2Motion {
            actor: actor.clone(),
            kind: Q2MotionKind::Stationary,
            velocity: body.velocity,
            angular_velocity: qa_core::math::vec3(0.0, 0.0, 0.0),
            gravity: 1.0,
            gravity_vector: qa_core::math::vec3(0.0, 0.0, -1.0),
            clip_mask: self.core.game.clip_mask(actor),
            owner: self.core.owner_of(&entity),
        })
    }

    /// Shared solidity record for an actor.
    pub fn collision(&self, actor: &OwnedActor) -> Result<SharedSolid, Q3SourceError> {
        let Some(entity) = self.core.records.native_by_actor(Some(actor.id())) else {
            return Err(Q3SourceError::RetiredCollision);
        };
        if !self.core.owns(actor) {
            return Err(Q3SourceError::RetiredCollision);
        }
        let guard = entity.borrow();
        let (solid, model) = if guard.r.contents == 0 {
            (SharedSolidKind::None, None)
        } else if guard.r.contents == TRIGGER_CONTENTS {
            (SharedSolidKind::Trigger, None)
        } else {
            match guard.r.model {
                EntityCollisionModel::Inline { index } => (SharedSolidKind::Brush, Some(index)),
                _ => (SharedSolidKind::Box, None),
            }
        };
        drop(guard);
        Ok(SharedSolid {
            solid,
            model,
            family: PhysicsFamily::Q3,
            owner: self.core.owner_of(&entity),
            monster: false,
            dead_monster: false,
            q1_corpse: false,
            item: false,
        })
    }

    /// Fixed invulnerability pose, if the shell is still up.
    pub fn fixed_pose(
        &self,
        actor: &OwnedActor,
        postures: &Q3Postures,
    ) -> Result<Option<FixedMovementPose>, Q3SourceError> {
        let entity = self.core.player(actor)?;
        let has_client = entity.borrow().client.clone();
        let Some(client) = has_client else {
            return Err(Q3SourceError::MissingPoseClient);
        };
        let expired = client.borrow().invulnerability_time <= self.core.now();
        if expired {
            client.borrow_mut().ps.pm_flags &= !INVULEXPAND;
            return Ok(None);
        }
        self.core.game.update_client_powerups(actor);
        self.core.game.expand_invulnerability(actor);
        let expanded = client.borrow().ps.pm_flags & INVULEXPAND != 0;
        Ok(Some(q3_invulnerability_pose(expanded, postures)))
    }

    /// Advance movement timers for a command.
    pub fn advance_movement(&self, actor: &OwnedActor, milliseconds: i32) -> Result<(), Q3SourceError> {
        let entity = self.core.player(actor)?;
        let client = entity
            .borrow()
            .client
            .clone()
            .ok_or(Q3SourceError::MissingMovementClient)?;
        {
            let mut guard = client.borrow_mut();
            let mut flags = guard.ps.pm_flags as u32;
            drop_q3_movement_timers(&mut guard.ps.pm_time, &mut flags, milliseconds);
            guard.ps.pm_flags = flags as i32;
        }
        self.core.publish_client(actor, &client);
        Ok(())
    }

    /// End-of-command equipment update.
    pub fn end_command(&self, actor: &OwnedActor, milliseconds: i32) -> Result<(), Q3SourceError> {
        if !self.owns_equipment() {
            return Ok(());
        }
        let entity = self.core.player(actor)?;
        let _ = entity;
        let health = self
            .core
            .host
            .combat()
            .read(actor.id())
            .map(|combat| combat.health)
            .unwrap_or(0);
        if health <= 0 {
            return Ok(());
        }
        self.core.run(|core| {
            core.game.client_timer_actions(actor, milliseconds, false, None);
            Ok(())
        })
    }

    /// Whether invulnerability blocks a damage request.
    pub fn blocks_damage(&self, request: &DamageRequest) -> Result<bool, Q3SourceError> {
        self.core.assert_open()?;
        let Some(target) = self.core.records.native_by_actor(Some(&request.target)) else {
            return Ok(false);
        };
        let _ = target;
        let means_of_death = match &request.attack.cause {
            AttackCause::Q3 { means_of_death, .. } => *means_of_death,
            _ => -1,
        };
        self.core.run(|core| {
            Ok(core
                .game
                .invulnerability_blocks(&request.target, request.direction, request.point, means_of_death))
        })
    }

    /// Respawn a player, preserving session, persistant words, event
    /// sequence, and ping.
    pub fn respawn(&self, actor: &ActorId) -> Result<(), Q3SourceError> {
        self.core.assert_open()?;
        let entity = self.core.records.native_by_actor(Some(actor));
        let client = entity.as_ref().and_then(|entity| entity.borrow().client.clone());
        let (Some(entity), Some(client)) = (entity, client) else {
            return Ok(());
        };
        self.release_hook(actor)?;
        let owned = self.core.actor_of_slot(entity.borrow().slot);
        if let Some(owned) = owned {
            self.core.return_persistent(&owned);
        }
        self.core.run(|core| {
            let fresh = GameClient::new(core.host.product(), None, None);
            {
                let mut guard = client.borrow_mut();
                let persistant: Vec<i32> = (0..guard.ps.persistant.len())
                    .map(|index| guard.ps.persistant.get(index))
                    .collect();
                let event_sequence = guard.ps.event_sequence;
                let ping = guard.ps.ping;
                guard.ps.copy_from(&fresh.ps, AuthorityStores::PreserveAuthority);
                for (index, value) in persistant.iter().enumerate() {
                    guard.ps.persistant.set(index, *value);
                }
                guard.ps.event_sequence = event_sequence;
                guard.ps.ping = ping;
                guard.noclip = fresh.noclip;
                guard.invulnerability_time = fresh.invulnerability_time;
                guard.ammo_times = fresh.ammo_times;
            }
            let slot = entity.borrow().slot;
            client.borrow_mut().ps.client_num = slot as i32;
            core.client_effects.borrow_mut().remove(actor);
            core.refresh(&entity)?;
            Ok(())
        })
    }

    /// Gauntlet hit check.
    pub fn gauntlet_hit(&self, actor: &OwnedActor) -> Result<bool, Q3SourceError> {
        let entity = self.core.player(actor)?;
        let _ = entity;
        self.core.run(|core| Ok(core.game.check_gauntlet_attack(actor)))
    }

    /// Run a weapon command.
    pub fn command(&self, actor: &OwnedActor, attack: bool, selected: bool, alive: bool) -> Result<(), Q3SourceError> {
        let entity = self.core.player(actor)?;
        if entity.borrow().client.is_none() {
            return Err(Q3SourceError::MissingClient);
        }
        self.core.run(|core| {
            core.game.command(actor, attack, selected, alive);
            Ok(())
        })
    }

    /// Release a grappling hook.
    pub fn release_hook(&self, actor: &ActorId) -> Result<(), Q3SourceError> {
        self.core.assert_open()?;
        let entity = self.core.records.native_by_actor(Some(actor));
        let hooked = entity.as_ref().is_some_and(|entity| entity.borrow().client.is_some());
        if !hooked {
            return Ok(());
        }
        let Some(owned) = self.core.host.actors().resolve_owned(actor) else {
            return Ok(());
        };
        if self.core.game.hook_free(&owned) {
            self.core.publish();
        }
        Ok(())
    }

    /// Grapple pull point, if pulling.
    pub fn grapple_point(&self, actor: &ActorId) -> Option<Vec3> {
        let entity = self.core.records.native_by_actor(Some(actor))?;
        let client = entity.borrow().client.clone()?;
        let guard = client.borrow();
        if guard.ps.pm_flags & GRAPPLE_PULL != 0 {
            Some(guard.ps.grapple_point)
        } else {
            None
        }
    }

    /// Grapple pull velocity, if pulling with a body and pose.
    pub fn pull(&self, actor: &OwnedActor) -> Option<Vec3> {
        let point = self.grapple_point(actor.id())?;
        let body = self.core.host.bodies().read(actor.id())?;
        let pose = self.core.host.player(actor.id())?;
        Some(qa_content::q3::base::game::grapple::q3_grapple_velocity(
            body.origin,
            point,
            qa_content::q3::foundation::player_pose::qvm_angle_vectors(pose.angles).forward,
        ))
    }

    /// Equipment-owned runtime slice.
    pub fn equipment(&self, actor: &OwnedActor) -> Result<Q3SelectedEquipmentState, Q3SourceError> {
        let entity = self.core.player(actor)?;
        let client = entity.borrow().client.clone().ok_or(Q3SourceError::MissingClient)?;
        let ps = client.borrow().ps.clone();
        let product = self.core.host.product();
        let (max_health, holdable_item, persistent_powerup_tag) = match product {
            Product::Baseq3 => {
                let StatSchema::Base(layout) = qa_content::q3::base::shared::definitions::stat_schema(product) else {
                    panic!("stat schema product mismatch");
                };
                (
                    ps.stats.get(layout.max_health as usize),
                    ps.stats.get(layout.holdable_item as usize),
                    Powerup::PwNone as i32,
                )
            }
            Product::Missionpack => {
                let StatSchema::Missionpack(layout) = qa_content::q3::base::shared::definitions::stat_schema(product)
                else {
                    panic!("stat schema product mismatch");
                };
                (
                    ps.stats.get(layout.max_health as usize),
                    ps.stats.get(layout.holdable_item as usize),
                    item_at(product, ps.stats.get(layout.persistent_powerup as usize))
                        .map(|item| item.tag())
                        .unwrap_or(Powerup::PwNone as i32),
                )
            }
        };
        let holdable_tag = item_at(product, holdable_item).map(|item| item.tag()).unwrap_or(0);
        Ok(Q3SelectedEquipmentState {
            max_health,
            persistent_powerup_tag,
            holdable_item,
            holdable_tag,
        })
    }
}

/// One source inventory row (donor `inventory` return element).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SourceInventoryEntry {
    /// Item id.
    pub item: ItemId,
    /// Display label.
    pub label: String,
    /// Count.
    pub count: i32,
    /// Usable (held holdable).
    pub usable: bool,
    /// Icon path, if any.
    pub icon: Option<String>,
}

impl Q3SelectedSource {
    /// Held and carried equipment rows.
    pub fn inventory(&self, actor: &OwnedActor) -> Result<Vec<Q3SourceInventoryEntry>, Q3SourceError> {
        if !self.owns_equipment() {
            return Ok(Vec::new());
        }
        let equipment = self.equipment(actor)?;
        let mut rows = Vec::new();
        for (index, item) in item_list(self.core.host.product()).iter().enumerate() {
            let held = item.kind.item_type() == ItemType::ItHoldable && index as i32 == equipment.holdable_item;
            let persistent = item.kind.item_type() == ItemType::ItPersistantPowerup
                && item.tag() == equipment.persistent_powerup_tag;
            if !held && !persistent {
                continue;
            }
            let (Some(class_name), Some(pickup_name)) = (item.class_name, item.pickup_name) else {
                continue;
            };
            rows.push(Q3SourceInventoryEntry {
                item: format!("q3:{class_name}"),
                label: pickup_name.to_string(),
                count: 1,
                usable: held,
                icon: item.icon.map(str::to_string),
            });
        }
        Ok(rows)
    }

    /// Firing delay for an actor in milliseconds.
    pub fn firing_delay(&self, actor: &OwnedActor, milliseconds: i32) -> Result<i32, Q3SourceError> {
        let entity = self.core.player(actor)?;
        let client = entity
            .borrow()
            .client
            .clone()
            .ok_or(Q3SourceError::MissingDelayClient)?;
        let persistent = if self.owns_equipment() {
            self.core.game.carried_powerup_tag(actor).unwrap_or(0)
        } else {
            0
        };
        if let Some(effects) = self.core.host.weapon_effects() {
            return Ok(effects.firing_delay(actor.id(), milliseconds, persistent));
        }
        let haste = client.borrow().ps.powerups.get(Powerup::PwHaste as usize) != 0;
        Ok(q3_weapon_delay(milliseconds, persistent, haste))
    }

    /// Client speed multiplier.
    pub fn speed_multiplier(&self, actor: &ActorId) -> f32 {
        let entity = self.core.records.native_by_actor(Some(actor));
        if entity
            .as_ref()
            .and_then(|entity| entity.borrow().client.clone())
            .is_none()
        {
            return 1.0;
        }
        self.core.game.speed_multiplier(actor)
    }

    /// Whether an original armor pickup is allowed.
    pub fn pickup_allowed(&self, offer: &OriginalPickupOffer) -> bool {
        if !self.owns_equipment() {
            return true;
        }
        if !matches!(
            offer.default_resource,
            Some(PickupResource::Protection {
                channel: ProtectionChannel::Regular
            })
        ) {
            return true;
        }
        let entity = self.core.records.native_by_actor(Some(&offer.recipient));
        if entity
            .as_ref()
            .and_then(|entity| entity.borrow().client.clone())
            .is_none()
        {
            return true;
        }
        if self.core.host.product() != Product::Missionpack {
            return true;
        }
        let Some(inventory) = self.core.player_inventory(&offer.recipient) else {
            return true;
        };
        if inventory.product() != Product::Missionpack {
            return true;
        }
        let tag = item_at(Product::Missionpack, inventory.persistent_powerup_index())
            .map(|item| item.tag())
            .unwrap_or(Powerup::PwNone as i32);
        if tag != Powerup::PwScout as i32 && tag != Powerup::PwGuard as i32 {
            return true;
        }
        can_q3_armor_be_grabbed(&inventory).unwrap_or(false)
    }

    /// Take an original equipment pickup.
    pub fn take_pickup(&self, offer: &SourcePickupDescriptor) -> Result<SourcePickupAdmission, Q3SourceError> {
        if !self.owns_equipment() {
            return Ok(SourcePickupAdmission::Native);
        }
        if offer.item.item_type != ItemType::ItHoldable && offer.item.item_type != ItemType::ItPersistantPowerup {
            return Ok(SourcePickupAdmission::Native);
        }
        let items = item_list(self.core.host.product());
        let Some(model_index) = items
            .iter()
            .position(|item| item.class_name == offer.item.class_name.as_deref())
        else {
            return Ok(SourcePickupAdmission::Rejected);
        };
        let Some(actor) = self.core.host.actors().resolve_owned(&offer.player_actor) else {
            return Ok(SourcePickupAdmission::Rejected);
        };
        if !self.core.host.actors().is_live(&offer.item_actor) {
            return Ok(SourcePickupAdmission::Rejected);
        }
        let entity = self.core.player(&actor)?;
        if entity.borrow().client.is_none() {
            return Err(Q3SourceError::MissingPickupClient);
        }
        let inventory = self.core.player_inventory(&offer.player_actor);
        let grabbed = inventory.as_ref().is_some_and(|inventory| {
            can_item_be_grabbed(
                offer.game_type,
                &PickupEntity {
                    model_index: model_index as i32,
                    model_index2: i32::from(offer.dropped),
                    generic1: offer.generic1,
                },
                inventory,
            )
            .unwrap_or(false)
        });
        if !grabbed {
            return Ok(SourcePickupAdmission::Rejected);
        }
        if self.core.project(&offer.item_actor)?.is_none() {
            return Ok(SourcePickupAdmission::Rejected);
        }
        let Some(item_actor) = self.core.host.actors().resolve_owned(&offer.item_actor) else {
            return Ok(SourcePickupAdmission::Rejected);
        };
        let handicap = client_info_value(&self.core.host.userinfo(actor.id()), "handicap");
        let respawn_seconds = self.core.game.take_pickup(
            &actor,
            &item_actor,
            model_index as i32,
            offer.count,
            offer.generic1,
            offer.dropped,
            offer.game_type,
            &handicap,
        );
        Ok(SourcePickupAdmission::Picked { respawn_seconds })
    }

    /// Release persistent powerups after a death.
    pub fn died(&self, actor: &ActorId) -> Result<(), Q3SourceError> {
        let entity = self.core.records.native_by_actor(Some(actor));
        if entity
            .as_ref()
            .and_then(|entity| entity.borrow().client.clone())
            .is_none()
        {
            return Ok(());
        }
        let Some(owned) = self.core.host.actors().resolve_owned(actor) else {
            return Ok(());
        };
        self.core.run(|core| {
            core.return_persistent(&owned);
            Ok(())
        })
    }

    /// Consume a held holdable.
    pub fn consume(&self, actor: &OwnedActor, item: i32) -> Result<(), Q3SourceError> {
        let entity = self.core.player(actor)?;
        let client = entity.borrow().client.clone().ok_or(Q3SourceError::HoldableMismatch)?;
        let product = self.core.host.product();
        let holdable_slot = match qa_content::q3::base::shared::definitions::stat_schema(product) {
            StatSchema::Base(layout) => layout.holdable_item,
            StatSchema::Missionpack(layout) => layout.holdable_item,
        };
        let current = client.borrow().ps.stats.get(holdable_slot as usize);
        let is_holdable =
            item_at(product, item).is_ok_and(|definition| definition.kind.item_type() == ItemType::ItHoldable);
        if current != item || !is_holdable {
            return Err(Q3SourceError::HoldableMismatch);
        }
        client.borrow_mut().ps.stats.set(holdable_slot as usize, 0);
        Ok(())
    }

    /// Grant a holdable by class or pickup name.
    pub fn give_holdable(&self, actor: &OwnedActor, name: &str) -> Result<bool, Q3SourceError> {
        if !self.owns_equipment() {
            return Ok(false);
        }
        let lowered = name.to_lowercase();
        let found = item_list(self.core.host.product()).iter().position(|item| {
            item.kind.item_type() == ItemType::ItHoldable
                && (item.class_name.is_some_and(|class| class.to_lowercase() == lowered)
                    || item.pickup_name.is_some_and(|pickup| pickup.to_lowercase() == lowered))
        });
        let Some(index) = found else {
            return Ok(false);
        };
        let entity = self.core.player(actor)?;
        let _ = entity;
        self.core.run(|core| {
            core.game.give_holdable(actor, index as i32);
            Ok(())
        })?;
        Ok(true)
    }

    /// Restore the equipment-owned runtime slice.
    pub fn restore_equipment(&self, actor: &OwnedActor, saved: &Q3SelectedEquipmentState) -> Result<(), Q3SourceError> {
        self.core.assert_open()?;
        self.core.host.actors().assert_owned(actor)?;
        let entity = self.core.records.native_by_actor(Some(actor.id()));
        let client = entity.as_ref().and_then(|entity| entity.borrow().client.clone());
        let Some(client) = client else {
            return Err(Q3SourceError::MissingRestoredClient);
        };
        let product = self.core.host.product();
        let mismatch = match item_at(product, saved.holdable_item) {
            Ok(item) => item.kind.item_type() != ItemType::ItHoldable || item.tag() != saved.holdable_tag,
            Err(_) => true,
        };
        if saved.holdable_item != 0 && mismatch {
            return Err(Q3SourceError::RestoredHoldableMismatch);
        }
        let holdable_slot = match qa_content::q3::base::shared::definitions::stat_schema(product) {
            StatSchema::Base(layout) => layout.holdable_item,
            StatSchema::Missionpack(layout) => layout.holdable_item,
        };
        client
            .borrow_mut()
            .ps
            .stats
            .set(holdable_slot as usize, saved.holdable_item);
        Ok(())
    }

    /// Fire a weapon.
    pub fn fire(&self, actor: &OwnedActor, weapon: i32, _input: &WeaponStepInput) -> Result<(), Q3SourceError> {
        let entity = self.core.player(actor)?;
        entity.borrow_mut().s.weapon = weapon;
        {
            let client = entity.borrow().client.clone().ok_or(Q3SourceError::MissingClient)?;
            client.borrow_mut().ps.weapon = weapon;
        }
        entity.borrow_mut().s.event = qa_world::movement::q3::constants::entity_event::FIRE_WEAPON;
        self.core
            .event_times
            .borrow_mut()
            .insert(actor.id().clone(), self.core.now());
        let damage_factor = self.core.host.weapon_effects().map(|effects| {
            let doubler =
                self.owns_equipment() && self.core.game.carried_powerup_tag(actor) == Some(Powerup::PwDoubler as i32);
            effects.damage_factor(actor.id()) * (f32::from(doubler) + 1.0)
        });
        self.core.run(|core| {
            core.game.fire_weapon(actor, weapon, damage_factor);
            Ok(())
        })
    }

    /// Use a holdable.
    pub fn use_holdable(&self, actor: &OwnedActor, event: i32) -> Result<(), Q3SourceError> {
        let entity = self.core.player(actor)?;
        let _ = entity;
        let host = self.core.host.clone();
        let game = self.core.game.clone();
        self.core.run(|core| {
            if let Some(entity) = core.records.native_by_actor(Some(actor.id())) {
                entity.borrow_mut().s.event = event;
            }
            core.event_times.borrow_mut().insert(actor.id().clone(), core.now());
            game.use_holdable(actor, event, &|player| {
                host.drop_objectives(player);
                let spawn = host.spawn_point(player);
                game.teleport_player(player, spawn.origin, spawn.angles);
            });
            Ok(())
        })
    }

    /// Publish original shared callbacks before the next destination
    /// client command.
    pub fn synchronize(&self) -> Result<(), Q3SourceError> {
        self.core.assert_open()?;
        self.core.publish();
        Ok(())
    }

    /// Pool presentation state for saves and baselines.
    pub fn source_state(&self) -> Q3SourcePresentationState {
        self.core.game.pool_presentation_state()
    }

    /// Pool models for the presentation path.
    pub fn presentations(&self) -> Vec<SimulationPresentation> {
        self.core.game.pool_models()
    }
}

impl Q3SelectedSource {
    /// Project an actor into the source-slot view (shared with the
    /// legacy import).
    pub(crate) fn project_actor(&self, actor: &ActorId) -> Result<Option<EntityRef>, Q3SourceError> {
        self.core.project(actor)
    }

    /// Capture the source save words.
    pub fn capture(&self) -> Result<SaveJson, Q3SourceError> {
        self.core.assert_open()?;
        if self.core.depth.get() != 0 {
            return Err(Q3SourceError::SaveDuringCall);
        }
        self.core.bind_world()?;
        let provider = self.core.host.provider();
        let product = match self.core.host.product() {
            Product::Baseq3 => "baseq3",
            Product::Missionpack => "missionpack",
        };
        let save = self.core.game.capture_game();
        let published: Vec<SaveJson> = self
            .core
            .published
            .borrow()
            .iter()
            .map(|(actor, event)| {
                obj(vec![
                    ("actor", write_saved_actor(SavedActorId::from(actor))),
                    ("event", int(i64::from(event.event))),
                    ("time", int(i64::from(event.time))),
                ])
            })
            .collect();
        Ok(obj(vec![
            ("version", int(1)),
            ("provider", str(&format!("{}:{}", provider.namespace, provider.name))),
            ("product", str(product)),
            ("graph", save.graph),
            ("bridge", content_save_to_world(&self.core.bridge.capture_save_state())),
            ("missiles", save.missiles),
            ("personalPortal", save.portal.unwrap_or(SaveJson::Null)),
            ("random", int(i64::from(self.core.random.borrow().seed()))),
            ("published", arr(published)),
        ]))
    }

    /// Restore the source save words. The runtime resolves saved actors
    /// (the registry has no saved-actor lookup).
    pub fn restore(
        &self,
        value: &SaveJson,
        resolve: &dyn Fn(SavedActorId) -> Option<OwnedActor>,
        reference: &dyn Fn(SavedActorId) -> ActorId,
    ) -> Result<(), Q3SourceError> {
        self.core.assert_open()?;
        if self.core.depth.get() != 0
            || !self.core.projected.borrow().is_empty()
            || !self.core.executing.borrow().is_empty()
        {
            return Err(Q3SourceError::RestoreRequiresUnused);
        }
        let reader = SaveReader::at(value, "selected Q3 source");
        reader.field("version").literal_i64(1)?;
        let provider = reader.field("provider").string()?;
        let host_provider = self.core.host.provider();
        if provider != format!("{}:{}", host_provider.namespace, host_provider.name) {
            return Err(reader.fail("Saved Q3 source provider changed").into());
        }
        let product_field = reader.field("product");
        let product = if product_field.is_missing() {
            "missionpack".to_string()
        } else {
            product_field.choice_str(&["baseq3", "missionpack"])?
        };
        let product_matches = match self.core.host.product() {
            Product::Baseq3 => product == "baseq3",
            Product::Missionpack => product == "missionpack",
        };
        if !product_matches {
            return Err(reader.fail("Saved Q3 equipment product changed").into());
        }
        let graph = reader
            .field("graph")
            .value
            .ok_or_else(|| reader.fail("expected a value"))?;
        self.core
            .game
            .restore_graph(&self.core.records, graph, resolve, reference)?;
        let bridge = reader
            .field("bridge")
            .value
            .ok_or_else(|| reader.fail("expected a value"))?;
        let bridge = world_save_to_content(bridge);
        self.core
            .bridge
            .restore_save_state(&bridge)
            .map_err(|error| reader.fail(&error.to_string()))?;
        let portal = reader.field("personalPortal");
        if self.core.host.product() == Product::Baseq3 {
            if !matches!(portal.value, Some(SaveJson::Null)) {
                return Err(reader.fail("Base Q3 equipment has missionpack portal state").into());
            }
        } else if let Some(value) = portal.value {
            self.core.game.restore_portal(value)?;
        }
        let missiles = reader
            .field("missiles")
            .value
            .ok_or_else(|| reader.fail("expected a value"))?;
        self.core.game.restore_missiles(missiles, reference)?;
        let seed = reader.field("random").integer(i64::MIN)?;
        self.core.random.borrow_mut().reset(seed as i32);
        reader.field("published").list(|entry| -> Result<(), WorldError> {
            let actor = resolve(read_saved_actor(entry.field("actor"))?)
                .ok_or_else(|| entry.fail("Published event has no live selected source actor"))?;
            if self.core.records.native_by_actor(Some(actor.id())).is_none() {
                return Err(entry.fail("Published event has no live selected source actor"));
            }
            if self.core.published.borrow().contains_key(actor.id()) {
                return Err(reader.fail("Duplicate published source event"));
            }
            let event = entry.field("event").integer(i64::MIN)? as i32;
            let time = entry.field("time").integer(i64::MIN)? as i32;
            self.core
                .published
                .borrow_mut()
                .insert(actor.id().clone(), PublishedEvent { event, time });
            Ok(())
        })?;
        let world = self
            .core
            .records
            .native_by_actor(Some(self.core.host.world_actor().id()));
        if world.map(|entity| entity.borrow().slot) != Some(WORLD_SLOT) {
            return Err(reader
                .fail("Restored selected Q3 world actor differs from the current map")
                .into());
        }
        for (slot, record) in self.core.records.capture_ownership().iter().enumerate() {
            let Some(actor) = record.actor.clone() else {
                continue;
            };
            if record.borrowed && slot != WORLD_SLOT {
                self.core
                    .projected
                    .borrow_mut()
                    .insert(actor.id().clone(), (actor.clone(), slot));
                if let Some(entity) = self.core.records.get(slot) {
                    if let Some(client) = entity.borrow().client.clone() {
                        let effects = self.core.effects_of(&client);
                        self.core
                            .client_effects
                            .borrow_mut()
                            .insert(actor.id().clone(), effects);
                    }
                }
            } else if !record.borrowed {
                self.core.track(&actor);
            }
        }
        *self.core.restored_presentation.borrow_mut() = Some(self.source_state());
        self.core.revision.set(self.core.revision.get() + 1);
        Ok(())
    }

    /// Restore a legacy ballistics save into an unused base source.
    pub fn restore_legacy(
        &self,
        saved: &super::q3_source_legacy::LegacyQ3Source,
        clients: &[super::q3_source_legacy::LegacyQ3Client],
    ) -> Result<(), Q3SourceError> {
        self.core.assert_open()?;
        if self.core.depth.get() != 0
            || !self.core.projected.borrow().is_empty()
            || !self.core.executing.borrow().is_empty()
            || self.core.host.product() != Product::Baseq3
        {
            return Err(Q3SourceError::LegacyRequiresUnused);
        }
        self.core.bind_world()?;
        for client in clients {
            let checkpoint = &client.state;
            let arsenal_matches = checkpoint.runtime.product == Product::Baseq3
                && checkpoint.arsenal.provider == self.core.host.provider()
                && matches!(
                    checkpoint.arsenal.state,
                    qa_world::movement::types::WeaponState::Q3 { .. }
                );
            if !arsenal_matches {
                return Err(Q3SourceError::LegacyArsenalMismatch);
            }
            let entity = self.core.player(&client.actor)?;
            if entity.borrow().client.is_none() {
                return Err(Q3SourceError::LegacyMissingClient);
            }
            self.restore_equipment(
                &client.actor,
                &Q3SelectedEquipmentState {
                    max_health: checkpoint.runtime.max_health as i32,
                    persistent_powerup_tag: checkpoint.runtime.persistent_powerup_tag,
                    holdable_item: checkpoint.runtime.holdable_item,
                    holdable_tag: checkpoint.runtime.holdable_tag,
                },
            )?;
            let qa_world::movement::types::WeaponState::Q3 {
                source_weapon,
                state,
                time_milliseconds,
            } = checkpoint.arsenal.state.clone()
            else {
                return Err(Q3SourceError::LegacyArsenalMismatch);
            };
            {
                let client_ref = entity
                    .borrow()
                    .client
                    .clone()
                    .ok_or(Q3SourceError::LegacyMissingClient)?;
                let mut guard = client_ref.borrow_mut();
                guard.ps.weapon = source_weapon;
                guard.ps.weapon_state = state;
                guard.ps.weapon_time = time_milliseconds;
                guard.ps.torso_anim = checkpoint.torso_animation;
                guard.ps.event_sequence = checkpoint.runtime.event_sequence;
                guard.ps.entity_event_sequence = checkpoint.runtime.event_sequence;
            }
            entity.borrow_mut().s.weapon = source_weapon;
        }
        super::q3_source_legacy::restore_legacy_q3_source(self, saved)?;
        for state in &saved.projectiles {
            let entity = self.core.records.native_by_actor(Some(&state.base.actor));
            let Some(entity) = entity else {
                return Err(Q3SourceError::LegacyImportLost);
            };
            let Some(owned) = self.core.host.actors().resolve_owned(&state.base.actor) else {
                return Err(Q3SourceError::LegacyImportLost);
            };
            let _ = entity;
            self.core.track(&owned);
        }
        for event in self.core.game.poll_entity_events() {
            if event.state.event == 0 {
                continue;
            }
            if let Some(entity) = self.core.records.native_by_actor(Some(event.actor.id())) {
                entity.borrow_mut().s = event.state.clone();
            }
            self.core
                .event_times
                .borrow_mut()
                .insert(event.actor.id().clone(), event.time);
            self.core.published.borrow_mut().insert(
                event.actor.id().clone(),
                PublishedEvent {
                    event: event.state.event,
                    time: event.time,
                },
            );
        }
        *self.core.restored_presentation.borrow_mut() = Some(self.source_state());
        self.core.revision.set(self.core.revision.get() + 1);
        Ok(())
    }

    /// Close the source, returning persistent powerups and releasing
    /// owned actors.
    pub fn close(&self) -> Result<(), Q3SourceError> {
        if self.core.closed.get() {
            return Ok(());
        }
        if self.core.depth.get() != 0 {
            return Err(Q3SourceError::CloseDuringCall);
        }
        let max_clients = self.core.host.max_clients();
        let owned: Vec<OwnedActor> = self
            .core
            .projected
            .borrow()
            .values()
            .filter(|(_, slot)| *slot < max_clients)
            .map(|(actor, _)| actor.clone())
            .collect();
        for actor in &owned {
            self.core.return_persistent(actor);
        }
        self.core.closed.set(true);
        for record in self.core.records.capture_ownership() {
            if let Some(actor) = record.actor {
                if !record.borrowed && self.core.host.actors().is_live(actor.id()) {
                    self.core.host.actors().release(&actor);
                }
            }
        }
        self.core.game.close_missiles();
        self.core.records.close();
        if let Some(unobserve) = self.core.unobserve.borrow_mut().take() {
            unobserve();
        }
        self.core.projected.borrow_mut().clear();
        self.core.executing.borrow_mut().clear();
        self.core.published.borrow_mut().clear();
        self.core.client_effects.borrow_mut().clear();
        self.core.event_times.borrow_mut().clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use qa_content::q3::base::records::{
        ActorCallbacks, ArmorState, CombatState, DamageAdmissionFn, DamageOutcome,
        InventoryEntry as RecordInventoryEntry, PoweredProtectionState, RegularArmorState,
    };
    use qa_content::q3::base::shared::definitions::stat_schema;
    use qa_content::q3::base::world::TraceContact;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec3, Bounds};

    use super::*;

    /// Release watcher list.
    type ReleaseWatchers = Vec<Box<dyn Fn(&OwnedActor)>>;
    /// Registered steppers per actor.
    type StepperTable = HashMap<ActorId, Vec<Box<dyn Fn(i32, i32)>>>;
    /// One actor's steppers, drained for driving.
    type StepperEntry = (ActorId, Vec<Box<dyn Fn(i32, i32)>>);

    struct MockActors {
        owner: IdentityOwner,
        owned: RefCell<HashMap<ActorId, OwnedActor>>,
        live: RefCell<HashSet<ActorId>>,
        generation: Cell<u32>,
        watchers: RefCell<ReleaseWatchers>,
    }

    impl MockActors {
        fn new() -> Self {
            MockActors {
                owner: IdentityOwner::create("q3-source-test").unwrap(),
                owned: RefCell::new(HashMap::new()),
                live: RefCell::new(HashSet::new()),
                generation: Cell::new(1),
                watchers: RefCell::new(Vec::new()),
            }
        }

        fn allocate(&self, provider: &ProviderId, slot: u32, definition: &str) -> OwnedActor {
            let id = self.owner.actor(slot, self.generation.get());
            self.generation.set(self.generation.get() + 1);
            let owned = self.owner.owned_actor(&id, provider.clone()).unwrap();
            self.owned.borrow_mut().insert(id.clone(), owned.clone());
            self.live.borrow_mut().insert(id);
            let _ = definition;
            owned
        }
    }

    impl Q3SessionActors for MockActors {
        fn assert_owned(&self, actor: &OwnedActor) -> Result<(), Q3BaseError> {
            if self.live.borrow().contains(actor.id()) && self.owned.borrow().get(actor.id()) == Some(actor) {
                Ok(())
            } else {
                Err(Q3BaseError::Invalid("foreign actor".to_string()))
            }
        }

        fn allocate_at_source(&self, owner: &ProviderId, source_slot: usize, definition: &str) -> OwnedActor {
            self.allocate(owner, source_slot as u32, definition)
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            self.live.borrow().contains(actor)
        }

        fn on_release(&self, callback: Box<dyn Fn(&OwnedActor)>) -> Box<dyn Fn()> {
            self.watchers.borrow_mut().push(callback);
            Box::new(|| {})
        }

        fn release(&self, actor: &OwnedActor) {
            self.live.borrow_mut().remove(actor.id());
            for watcher in self.watchers.borrow().iter() {
                watcher(actor);
            }
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            if self.live.borrow().contains(actor) {
                self.owned.borrow().get(actor).cloned()
            } else {
                None
            }
        }
    }

    struct MockBodies {
        states: RefCell<HashMap<ActorId, BodyState>>,
    }

    impl Q3SessionBodies for MockBodies {
        fn create(&self, actor: &OwnedActor, initial: BodyState) {
            self.states.borrow_mut().insert(actor.id().clone(), initial);
        }

        fn read(&self, actor: &ActorId) -> Option<BodyState> {
            self.states.borrow().get(actor).cloned()
        }

        fn write(&self, actor: &OwnedActor, state: BodyState) {
            self.states.borrow_mut().insert(actor.id().clone(), state);
        }

        fn linked(&self, actor: &ActorId) -> Option<qa_world::body::LinkedBody> {
            self.states.borrow().get(actor).map(|state| qa_world::body::LinkedBody {
                actor: actor.clone(),
                state: state.clone(),
                absolute_bounds: state.bounds,
                link_count: 1,
            })
        }

        fn link(&self, _actor: &OwnedActor, _origin: Option<Vec3>) {}

        fn unlink(&self, _actor: &OwnedActor) {}
    }

    struct MockCombat {
        states: RefCell<HashMap<ActorId, CombatState>>,
    }

    impl Q3SessionCombat for MockCombat {
        fn create(&self, actor: &OwnedActor, initial: CombatState, _admit_damage: Option<DamageAdmissionFn>) {
            self.states.borrow_mut().insert(actor.id().clone(), initial);
        }

        fn read(&self, actor: &ActorId) -> Option<CombatState> {
            self.states.borrow().get(actor).cloned()
        }

        fn set_health(&self, actor: &OwnedActor, health: i32) {
            if let Some(state) = self.states.borrow_mut().get_mut(actor.id()) {
                state.health = health;
            }
        }

        fn set_can_take_damage(&self, _actor: &OwnedActor, _can_take_damage: bool) {}

        fn set_regular_points(
            &self,
            _actor: &OwnedActor,
            _points: i32,
            _initial: qa_content::q3::base::records::RegularArmorState,
        ) {
        }

        fn bind_damage_admission(&self, _actor: &OwnedActor, _admit_damage: DamageAdmissionFn) {}

        fn apply(&self, _input: DamageRequest) -> DamageOutcome {
            panic!("unexpected damage application in source tests")
        }
    }

    struct MockInventory {
        counts: RefCell<HashMap<(ActorId, ItemId), i32>>,
    }

    impl Q3SessionInventory for MockInventory {
        fn create(&self, _actor: &OwnedActor, _entries: Vec<RecordInventoryEntry>) {}

        fn has(&self, _actor: &ActorId) -> bool {
            false
        }

        fn count(&self, actor: &ActorId, item: &ItemId) -> i32 {
            self.counts
                .borrow()
                .get(&(actor.clone(), item.clone()))
                .copied()
                .unwrap_or(0)
        }

        fn configure(&self, _actor: &OwnedActor, _item: &ItemId, _count: i32, _capacity: i32) {}
    }

    struct MockCallbacks;

    impl Q3ActorCallbacks for MockCallbacks {
        fn bind(&self, _actor: &OwnedActor, _callbacks: ActorCallbacks) {}
    }

    struct MockWorldHost {
        bodies: Rc<MockBodies>,
        collisions: RefCell<HashMap<ActorId, ActorCollision>>,
    }

    impl Q3WorldAdapterHost for MockWorldHost {
        fn trace_scene(&self, query: &Q3TraceQuery) -> Q3TraceResult {
            Q3TraceResult {
                fraction: 1.0,
                end: query.end,
                hit: qa_content::q3::base::world_adapter::Q3TraceHit::None,
                contact: TraceContact::None,
                start_solid: false,
                all_solid: false,
                contents: 0,
                surface_flags: 0,
            }
        }

        fn point_contents_scene(&self, _query: &Q3TraceQuery, _point: Vec3) -> i32 {
            0
        }

        fn query_actors(&self, _bounds: Bounds) -> Vec<ActorId> {
            Vec::new()
        }

        fn spatial_collision(&self, actor: &ActorId) -> Option<ActorCollision> {
            self.collisions.borrow().get(actor).cloned()
        }

        fn body_state(&self, actor: &ActorId) -> Option<BodyState> {
            self.bodies.read(actor)
        }

        fn linked_body(&self, _actor: &ActorId) -> Option<qa_world::body::LinkedBody> {
            None
        }

        fn set_collision(&self, actor: &OwnedActor, collision: ActorCollision) {
            self.collisions.borrow_mut().insert(actor.id().clone(), collision);
        }

        fn link_body(&self, _actor: &OwnedActor, _origin: Option<Vec3>) {}

        fn unlink_body(&self, _actor: &OwnedActor) {}

        fn curves(&self) -> bool {
            false
        }

        fn player_curve_clip(&self) -> bool {
            false
        }

        fn geometry_trace_start_solid(&self, _query: &Q3TraceQuery, _model: i32, _origin: Vec3, _angles: Vec3) -> bool {
            false
        }

        fn body_trace_start_solid(
            &self,
            _query: &Q3TraceQuery,
            _body: &BodyState,
            _collision: &ActorCollision,
        ) -> bool {
            false
        }
    }

    struct MockStrings {
        values: RefCell<HashMap<usize, String>>,
    }

    impl ConfigStringStore for MockStrings {
        fn get(&self, index: usize) -> String {
            self.values.borrow().get(&index).cloned().unwrap_or_default()
        }

        fn set(&mut self, index: usize, value: &str) {
            self.values.borrow_mut().insert(index, value.to_string());
        }
    }

    struct FixedEffects {
        factor: f32,
        delay: i32,
    }

    impl Q3SourceWeaponEffects for FixedEffects {
        fn damage_factor(&self, _actor: &ActorId) -> f32 {
            self.factor
        }

        fn firing_delay(&self, _actor: &ActorId, _milliseconds: i32, _persistent: i32) -> i32 {
            self.delay
        }
    }

    struct MockHost {
        actors: Rc<MockActors>,
        bodies: Rc<MockBodies>,
        combat: Rc<MockCombat>,
        inventory: Rc<MockInventory>,
        callbacks: Rc<MockCallbacks>,
        world: Rc<MockWorldHost>,
        strings: Rc<RefCell<MockStrings>>,
        poses: RefCell<HashMap<ActorId, Q3SelectedClientPose>>,
        steppers: RefCell<StepperTable>,
        events: RefCell<Vec<(ActorId, i32, i32)>>,
        changed: RefCell<Vec<ActorId>>,
        provider: ProviderId,
        product: Product,
        equipment: Q3EquipmentKind,
        world_actor: OwnedActor,
        now: Cell<i32>,
        effects: Option<Rc<FixedEffects>>,
    }

    impl Q3SelectedSourceHost for MockHost {
        fn actors(&self) -> Rc<dyn Q3SessionActors> {
            self.actors.clone()
        }

        fn bodies(&self) -> Rc<dyn Q3SessionBodies> {
            self.bodies.clone()
        }

        fn combat(&self) -> Rc<dyn Q3SessionCombat> {
            self.combat.clone()
        }

        fn inventory(&self) -> Rc<dyn Q3SessionInventory> {
            self.inventory.clone()
        }

        fn callbacks(&self) -> Rc<dyn Q3ActorCallbacks> {
            self.callbacks.clone()
        }

        fn world_queries(&self) -> Rc<dyn Q3WorldAdapterHost> {
            self.world.clone()
        }

        fn combat_provider(&self) -> ProviderId {
            ProviderId::new("sim", "combat")
        }

        fn source_damage_modifier(&self) -> Option<SourceDamageModifier> {
            None
        }

        fn damage_powerup_owner(&self) -> Option<ProviderId> {
            None
        }

        fn inventory_provider(&self) -> ProviderId {
            ProviderId::new("sim", "inventory")
        }

        fn movement_provider(&self) -> ProviderId {
            ProviderId::new("sim", "movement")
        }

        fn armor_context(&self, _request: &DamageRequest) -> VictimArmorContext {
            panic!("unexpected armor context in source tests")
        }

        fn game_type(&self) -> i32 {
            0
        }

        fn friendly_fire(&self) -> bool {
            false
        }

        fn knockback(&self) -> f32 {
            1000.0
        }

        fn intermission_queued(&self) -> i32 {
            0
        }

        fn check_hurt_carrier(&self, _target: &ActorId, _attacker: &ActorId) {}

        fn provider(&self) -> ProviderId {
            self.provider.clone()
        }

        fn product(&self) -> Product {
            self.product
        }

        fn equipment_kind(&self) -> Q3EquipmentKind {
            self.equipment
        }

        fn weapon_effects(&self) -> Option<Rc<dyn Q3SourceWeaponEffects>> {
            self.effects
                .clone()
                .map(|effects| effects as Rc<dyn Q3SourceWeaponEffects>)
        }

        fn content(&self) -> ContentId {
            ContentId("q3:base:test:1".to_string())
        }

        fn configstrings(&self) -> Rc<RefCell<dyn ConfigStringStore>> {
            self.strings.clone() as Rc<RefCell<dyn ConfigStringStore>>
        }

        fn userinfo(&self, _actor: &ActorId) -> String {
            "\\name\\Tester\\handicap\\100".to_string()
        }

        fn max_clients(&self) -> usize {
            8
        }

        fn seed(&self) -> i32 {
            1234
        }

        fn now(&self) -> i32 {
            self.now.get()
        }

        fn world_actor(&self) -> OwnedActor {
            self.world_actor.clone()
        }

        fn player(&self, actor: &ActorId) -> Option<Q3SelectedClientPose> {
            self.poses.borrow().get(actor).cloned()
        }

        fn quad_factor(&self, _actor: Option<&ActorId>) -> f32 {
            3.0
        }

        fn proximity_timeout(&self) -> i32 {
            10000
        }

        fn model_index(&self, _path: &str) -> i32 {
            1
        }

        fn sound_index(&self, _path: &str) -> i32 {
            1
        }

        fn print(&self, _text: &str) {}

        fn execute(&self, actor: &OwnedActor, step: Box<dyn Fn(i32, i32)>) {
            self.steppers
                .borrow_mut()
                .entry(actor.id().clone())
                .or_default()
                .push(step);
        }

        fn event(&self, actor: &OwnedActor, state: EntityState, time: i32) {
            self.events.borrow_mut().push((actor.id().clone(), state.event, time));
        }

        fn client_changed(
            &self,
            actor: &OwnedActor,
            _before: &Q3SelectedClientEffects,
            _after: &Q3SelectedClientEffects,
        ) {
            self.changed.borrow_mut().push(actor.id().clone());
        }

        fn check_obelisk_attack(&self, _target: &ActorId, _attacker: &ActorId) -> bool {
            false
        }

        fn drop_objectives(&self, _actor: &OwnedActor) {}

        fn return_pickup(&self, _actor: &ActorId) {}

        fn spawn_point(&self, _actor: &OwnedActor) -> Q3SpawnPoint {
            Q3SpawnPoint {
                origin: vec3(0.0, 0.0, 0.0),
                angles: vec3(0.0, 0.0, 0.0),
            }
        }
    }

    struct MockGame {
        log: RefCell<Vec<String>>,
        free_slots: RefCell<Vec<i32>>,
        polled: RefCell<Vec<Q3SourceEntityEvent>>,
        synced: RefCell<Vec<OwnedActor>>,
        powerup_tag: Cell<Option<i32>>,
        clip: Cell<i32>,
        speed: Cell<f32>,
        hook_free: Cell<bool>,
        expire: Cell<bool>,
        run_owned: Cell<bool>,
        gauntlet: Cell<bool>,
        accuracy: Cell<bool>,
        blocked: Cell<bool>,
        take_respawn: Cell<i32>,
        config: RefCell<String>,
        world: RefCell<Option<OwnedActor>>,
        player: RefCell<Option<OwnedActor>>,
    }

    impl MockGame {
        fn new() -> Self {
            MockGame {
                log: RefCell::new(Vec::new()),
                free_slots: RefCell::new((MAX_CLIENTS as i32..1022).collect()),
                polled: RefCell::new(Vec::new()),
                synced: RefCell::new(Vec::new()),
                powerup_tag: Cell::new(None),
                clip: Cell::new(1),
                speed: Cell::new(1.0),
                hook_free: Cell::new(false),
                expire: Cell::new(true),
                run_owned: Cell::new(false),
                gauntlet: Cell::new(false),
                accuracy: Cell::new(false),
                blocked: Cell::new(false),
                take_respawn: Cell::new(30),
                config: RefCell::new("config".to_string()),
                world: RefCell::new(None),
                player: RefCell::new(None),
            }
        }

        fn note(&self, message: String) {
            self.log.borrow_mut().push(message);
        }

        fn has(&self, entry: &str) -> bool {
            self.log.borrow().iter().any(|line| line == entry)
        }
    }

    impl Q3SourceGame for MockGame {
        fn bind_random(&self, _random: Rc<RefCell<GameRandom>>) {
            self.note("bind_random".to_string());
        }

        fn missile_owner_of(&self, _actor: &ActorId) -> Option<ActorId> {
            None
        }

        fn think(&self, actor: &OwnedActor, _time: i32) -> Result<(), Q3SourceError> {
            self.note(format!("think:{}", actor.id().slot()));
            Ok(())
        }

        fn expire_events(&self, actor: &OwnedActor, _time: i32) -> bool {
            self.note(format!("expire:{}", actor.id().slot()));
            self.expire.get()
        }

        fn run_owned_missiles(&self, actor: &OwnedActor, _previous: i32, _time: i32) -> bool {
            self.note(format!("run_owned:{}", actor.id().slot()));
            self.run_owned.get()
        }

        fn hook_free(&self, actor: &OwnedActor) -> bool {
            self.note(format!("hook_free:{}", actor.id().slot()));
            self.hook_free.get()
        }

        fn poll_entity_events(&self) -> Vec<Q3SourceEntityEvent> {
            std::mem::take(&mut *self.polled.borrow_mut())
        }

        fn sync_records(&self, _records: &Q3EntityRecords) -> Vec<OwnedActor> {
            std::mem::take(&mut *self.synced.borrow_mut())
        }

        fn check_gauntlet_attack(&self, _actor: &OwnedActor) -> bool {
            self.note("gauntlet".to_string());
            self.gauntlet.get()
        }

        fn fire_weapon(&self, actor: &OwnedActor, weapon: i32, damage_factor: Option<f32>) {
            self.note(format!("fire:{}:{weapon}:{damage_factor:?}", actor.id().slot()));
        }

        fn carried_powerup_tag(&self, _actor: &OwnedActor) -> Option<i32> {
            self.powerup_tag.get()
        }

        fn clip_mask(&self, _actor: &OwnedActor) -> i32 {
            self.clip.get()
        }

        fn log_accuracy_hit(&self, _target: &ActorId, _attacker: &ActorId) -> bool {
            self.accuracy.get()
        }

        fn damage_feedback(&self, _target: &ActorId, _decision: &DamageDecision) {}

        fn foreign_damage_feedback(&self, _target: &ActorId, _owner: Option<&ActorId>, _decision: &DamageDecision) {}

        fn use_holdable(&self, actor: &OwnedActor, event: i32, _teleport: &dyn Fn(&OwnedActor)) {
            self.note(format!("use_holdable:{}:{event}", actor.id().slot()));
        }

        fn teleport_player(&self, _actor: &OwnedActor, _origin: Vec3, _angles: Vec3) {}

        fn take_pickup(
            &self,
            _player: &OwnedActor,
            _item_actor: &OwnedActor,
            _item_index: i32,
            _count: i32,
            _generic1: i32,
            _dropped: bool,
            _game_type: i32,
            _handicap: &str,
        ) -> i32 {
            self.note("take_pickup".to_string());
            self.take_respawn.get()
        }

        fn toss_client_persistent_powerup(
            &self,
            actor: &OwnedActor,
            _spawn: &dyn Fn() -> OwnedActor,
            _on_pickup: &dyn Fn(&OwnedActor),
        ) {
            self.note(format!("toss:{}", actor.id().slot()));
        }

        fn return_persistent_pickup(&self, _pickup: &OwnedActor) {}

        fn give_holdable(&self, actor: &OwnedActor, item_index: i32) {
            self.note(format!("give:{}:{item_index}", actor.id().slot()));
        }

        fn update_client_powerups(&self, _actor: &OwnedActor) {
            self.note("powerups".to_string());
        }

        fn client_timer_actions(
            &self,
            _actor: &OwnedActor,
            _milliseconds: i32,
            _ordinary_decay: bool,
            _ammo: Option<Vec<Q3MappedAmmoTimer>>,
        ) {
            self.note("timers".to_string());
        }

        fn expand_invulnerability(&self, _actor: &OwnedActor) {
            self.note("expand".to_string());
        }

        fn invulnerability_effect(&self, _target: &OwnedActor, _direction: Vec3, _point: Vec3) {}

        fn invulnerability_blocks(
            &self,
            _target: &ActorId,
            _direction: Vec3,
            _point: Vec3,
            _means_of_death: i32,
        ) -> bool {
            self.note("blocks".to_string());
            self.blocked.get()
        }

        fn speed_multiplier(&self, _actor: &ActorId) -> f32 {
            self.speed.get()
        }

        fn capture_game(&self) -> Q3GameSave {
            Q3GameSave {
                graph: SaveJson::Null,
                missiles: SaveJson::Null,
                portal: None,
            }
        }

        fn restore_graph(
            &self,
            records: &Q3EntityRecords,
            _graph: &SaveJson,
            _resolve: &dyn Fn(SavedActorId) -> Option<OwnedActor>,
            _reference: &dyn Fn(SavedActorId) -> ActorId,
        ) -> Result<(), Q3SourceError> {
            if let Some(world) = self.world.borrow().clone() {
                records.attach(WORLD_SLOT, world, false)?;
            }
            if let Some(player) = self.player.borrow().clone() {
                records.attach(0, player, true)?;
            }
            Ok(())
        }

        fn restore_portal(&self, _value: &SaveJson) -> Result<(), Q3SourceError> {
            Ok(())
        }

        fn restore_missiles(
            &self,
            _value: &SaveJson,
            _reference: &dyn Fn(SavedActorId) -> ActorId,
        ) -> Result<(), Q3SourceError> {
            Ok(())
        }

        fn close_missiles(&self) {
            self.note("close_missiles".to_string());
        }

        fn free_entity_slots(&self) -> Vec<i32> {
            self.free_slots.borrow().clone()
        }

        fn import_legacy(&self, import: &LegacyImport) -> Result<(), Q3SourceError> {
            self.note(format!("import:{}", import.projectiles.len()));
            Ok(())
        }

        fn client_presentation_config(&self, _actor: &OwnedActor, _userinfo: &str, _game_type: i32) -> String {
            self.config.borrow().clone()
        }

        fn pool_presentation_state(&self) -> Q3SourcePresentationState {
            Q3SourcePresentationState {
                product: Product::Baseq3,
                time: 0,
                entities: Vec::new(),
                clients: Vec::new(),
                configstrings: Vec::new(),
            }
        }

        fn pool_models(&self) -> Vec<SimulationPresentation> {
            Vec::new()
        }

        fn project_entity(&self, actor: &OwnedActor, slot: i32, is_player: bool) {
            self.note(format!("project:{}:{slot}:{is_player}", actor.id().slot()));
        }

        fn refresh_client_persistent(&self, actor: &OwnedActor, _pose: &Q3SelectedClientPose, _netname: &str) {
            self.note(format!("persistent:{}", actor.id().slot()));
        }

        fn command(&self, actor: &OwnedActor, attack: bool, selected: bool, alive: bool) {
            self.note(format!("command:{}:{attack}:{selected}:{alive}", actor.id().slot()));
        }
    }

    struct Fixture {
        host: Rc<MockHost>,
        game: Rc<MockGame>,
        source: Q3SelectedSource,
        world: OwnedActor,
        player: OwnedActor,
    }

    fn body_at(origin: Vec3) -> BodyState {
        BodyState {
            origin,
            angles: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            bounds: Bounds {
                min: vec3(-15.0, -15.0, -24.0),
                max: vec3(15.0, 15.0, 32.0),
            },
            ground: None,
        }
    }

    fn combat_at(health: i32) -> CombatState {
        CombatState {
            health,
            armor: ArmorState {
                regular: RegularArmorState::None,
                powered: PoweredProtectionState::None,
            },
            mass: 100,
            can_take_damage: true,
            invulnerable: false,
            no_knockback: false,
            team: None,
        }
    }

    fn pose() -> Q3SelectedClientPose {
        Q3SelectedClientPose {
            angles: vec3(10.0, 20.0, 0.0),
            view_height: 26,
            max_health: 100,
            team: Team::TeamFree,
            quad_until: 5000,
            haste_until: 0,
        }
    }

    fn setup() -> Fixture {
        setup_with_product(Product::Baseq3, None)
    }

    fn setup_with_product(product: Product, effects: Option<Rc<FixedEffects>>) -> Fixture {
        let provider = ProviderId::new("q3", "test");
        let actors = Rc::new(MockActors::new());
        let bodies = Rc::new(MockBodies {
            states: RefCell::new(HashMap::new()),
        });
        let combat = Rc::new(MockCombat {
            states: RefCell::new(HashMap::new()),
        });
        let world = actors.allocate(&provider, 100, "q3:world");
        let player = actors.allocate(&provider, 1, "q3:player");
        bodies
            .states
            .borrow_mut()
            .insert(world.id().clone(), body_at(vec3(0.0, 0.0, 0.0)));
        bodies
            .states
            .borrow_mut()
            .insert(player.id().clone(), body_at(vec3(8.0, 0.0, 0.0)));
        combat.states.borrow_mut().insert(player.id().clone(), combat_at(100));
        let mut poses = HashMap::new();
        poses.insert(player.id().clone(), pose());
        let host = Rc::new(MockHost {
            actors: actors.clone(),
            bodies: bodies.clone(),
            combat,
            inventory: Rc::new(MockInventory {
                counts: RefCell::new(HashMap::new()),
            }),
            callbacks: Rc::new(MockCallbacks),
            world: Rc::new(MockWorldHost {
                bodies: bodies.clone(),
                collisions: RefCell::new(HashMap::new()),
            }),
            strings: Rc::new(RefCell::new(MockStrings {
                values: RefCell::new(HashMap::new()),
            })),
            poses: RefCell::new(poses),
            steppers: RefCell::new(HashMap::new()),
            events: RefCell::new(Vec::new()),
            changed: RefCell::new(Vec::new()),
            provider,
            product,
            equipment: Q3EquipmentKind::Source,
            world_actor: world.clone(),
            now: Cell::new(1000),
            effects,
        });
        let game = Rc::new(MockGame::new());
        let source = Q3SelectedSource::new(host.clone(), game.clone());
        Fixture {
            host,
            game,
            source,
            world,
            player,
        }
    }

    impl Fixture {
        fn drive(&self, previous: i32, time: i32) {
            let steppers: Vec<StepperEntry> = self
                .host
                .steppers
                .borrow_mut()
                .iter_mut()
                .map(|(actor, steps)| (actor.clone(), std::mem::take(steps)))
                .collect();
            for (_, steps) in &steppers {
                for step in steps {
                    step(previous, time);
                }
            }
            for (actor, steps) in steppers {
                self.host.steppers.borrow_mut().insert(actor, steps);
            }
        }

        fn client(&self, slot: usize) -> ClientRef {
            self.source.records().client(slot)
        }
    }

    #[test]
    fn admits_player_and_projects_pose() {
        let fixture = setup();
        fixture.source.admit(&fixture.player).unwrap();
        assert!(fixture.source.live(fixture.player.id()));
        assert_eq!(fixture.source.actor(0).as_ref(), Some(fixture.player.id()));
        let client = fixture.client(0);
        let guard = client.borrow();
        assert_eq!(guard.ps.viewangles, vec3(10.0, 20.0, 0.0));
        assert_eq!(guard.ps.viewheight, 26);
        assert_eq!(guard.ps.stats.get(stat_max_health(Product::Baseq3)), 100);
        assert_eq!(guard.ps.powerups.get(Powerup::PwQuad as usize), 5000);
        drop(guard);
        assert_eq!(fixture.host.strings.borrow().get(544), "config");
        assert!(fixture.game.has("project:1:0:true"));
        assert!(fixture.game.has("persistent:1"));
        assert_eq!(fixture.source.generation(), 0);
        assert!(fixture.source.active());
    }

    #[test]
    fn steps_and_reports_client_changes() {
        let fixture = setup();
        fixture.source.admit(&fixture.player).unwrap();
        fixture.game.synced.borrow_mut().push(fixture.player.clone());
        fixture.source.synchronize().unwrap();
        assert!(fixture.host.steppers.borrow().contains_key(fixture.player.id()));
        fixture.drive(900, 1000);
        assert!(fixture.host.changed.borrow().is_empty());
        fixture.client(0).borrow_mut().ps.viewangles = vec3(30.0, 40.0, 0.0);
        fixture.drive(1000, 1100);
        assert_eq!(fixture.host.changed.borrow().as_slice(), [fixture.player.id().clone()]);
        assert!(fixture.game.has("think:1"));
    }

    #[test]
    fn fires_and_publishes_once() {
        use qa_world::movement::q3::constants::entity_event::FIRE_WEAPON;
        let fixture = setup();
        fixture.source.admit(&fixture.player).unwrap();
        let input = WeaponStepInput {
            actor: fixture.player.clone(),
            command: qa_world::movement::types::UserCommand::Q3(qa_world::movement::types::Q3UserCommand {
                server_time_milliseconds: 1000,
                angle_words: [0, 0, 0],
                buttons: 0,
                weapon: 0,
                forward_move: 0,
                right_move: 0,
                up_move: 0,
            }),
            frame: qa_core::time::FrameContext {
                frame: 1,
                phase: qa_core::time::FramePhase::ClientCommand,
                time: qa_core::time::SourceTime::Milliseconds(1000),
                elapsed: qa_core::time::SourceTime::Milliseconds(100),
            },
            arsenal: qa_world::movement::types::ArsenalState {
                provider: ProviderId::new("q3", "test"),
                active_weapon: None,
                ammo: Vec::new(),
                state: qa_world::movement::types::WeaponState::Q3 {
                    source_weapon: 2,
                    state: 0,
                    time_milliseconds: 0,
                },
            },
            animation: qa_world::movement::types::ActorAnimationState {
                provider: ProviderId::new("q3", "test"),
                state: qa_world::movement::types::AnimationState::Q3 {
                    legs: 0,
                    torso: 0,
                    legs_timer_milliseconds: 0,
                    torso_timer_milliseconds: 0,
                },
            },
            environment: qa_world::movement::types::MovementEnvironment::default(),
            gauntlet_hit: false,
        };
        fixture.source.fire(&fixture.player, 2, &input).unwrap();
        assert!(fixture.game.has("fire:1:2:None"));
        fixture.source.synchronize().unwrap();
        assert_eq!(
            fixture.host.events.borrow().as_slice(),
            [(fixture.player.id().clone(), FIRE_WEAPON, 1000)]
        );
        fixture.source.synchronize().unwrap();
        assert_eq!(fixture.host.events.borrow().len(), 1);
    }

    #[test]
    fn computes_motion_collision_and_equipment() {
        let fixture = setup();
        fixture.source.admit(&fixture.player).unwrap();
        let body = body_at(vec3(8.0, 0.0, 0.0));
        let motion = fixture.source.motion(&fixture.player, &body).unwrap();
        assert_eq!(motion.kind, Q2MotionKind::Stationary);
        assert_eq!(motion.velocity, vec3(0.0, 0.0, 0.0));
        assert_eq!(motion.gravity, 1.0);
        assert_eq!(motion.clip_mask, 1);
        assert_eq!(motion.owner.as_ref(), Some(fixture.player.id()));
        let solid = fixture.source.collision(&fixture.player).unwrap();
        assert_eq!(solid.solid, SharedSolidKind::None);
        assert_eq!(solid.family, PhysicsFamily::Q3);
        let equipment = fixture.source.equipment(&fixture.player).unwrap();
        assert_eq!(equipment.max_health, 100);
        assert_eq!(equipment.persistent_powerup_tag, Powerup::PwNone as i32);
        assert!(fixture.source.inventory(&fixture.player).unwrap().is_empty());
    }

    #[test]
    fn computes_delays_and_speed() {
        let fixture = setup();
        fixture.source.admit(&fixture.player).unwrap();
        assert_eq!(
            fixture.source.firing_delay(&fixture.player, 100).unwrap(),
            q3_weapon_delay(100, 0, false)
        );
        fixture.game.speed.set(1.5);
        assert_eq!(fixture.source.speed_multiplier(fixture.player.id()), 1.5);
        let stranger = fixture.host.owner_actor(50);
        assert_eq!(fixture.source.speed_multiplier(stranger.id()), 1.0);
    }

    #[test]
    fn guards_pickups() {
        let fixture = setup();
        fixture.source.admit(&fixture.player).unwrap();
        let offer = OriginalPickupOffer {
            recipient: fixture.player.id().clone(),
            pickup: fixture.player.id().clone(),
            source: ProviderId::new("q3", "test"),
            item: "q3:armor/shard".to_string(),
            default_resource: Some(PickupResource::Protection {
                channel: ProtectionChannel::Regular,
            }),
            count: qa_content::contract::PickupCount::Default,
            dropped: false,
            time: qa_core::time::SourceTime::Milliseconds(1000),
            cargo: Vec::new(),
            grant: None,
        };
        assert!(fixture.source.pickup_allowed(&offer));
        let native = SourcePickupDescriptor {
            item_actor: fixture.player.id().clone(),
            player_actor: fixture.player.id().clone(),
            item: qa_content::q3::base::game::entities::ItemDefinition {
                class_name: Some("item_health".to_string()),
                pickup_name: Some("Health".to_string()),
                quantity: 25,
                item_type: ItemType::ItHealth,
                tag: 0,
            },
            count: 1,
            generic1: 0,
            dropped: false,
            game_type: 0,
            weapon_respawn_seconds: 30,
            team_weapon_respawn_seconds: 30,
        };
        assert!(matches!(
            fixture.source.take_pickup(&native).unwrap(),
            SourcePickupAdmission::Native
        ));
    }

    #[test]
    fn advances_and_ends_commands() {
        let fixture = setup();
        fixture.source.admit(&fixture.player).unwrap();
        fixture.client(0).borrow_mut().ps.pm_time = 100;
        fixture.source.advance_movement(&fixture.player, 100).unwrap();
        assert_eq!(fixture.client(0).borrow().ps.pm_time, 0);
        fixture.source.end_command(&fixture.player, 100).unwrap();
        assert!(fixture.game.has("timers"));
    }

    #[test]
    fn poses_and_blocks() {
        let fixture = setup();
        fixture.source.admit(&fixture.player).unwrap();
        fixture.client(0).borrow_mut().invulnerability_time = 5000;
        fixture.client(0).borrow_mut().ps.pm_flags = INVULEXPAND;
        let postures = Q3Postures {
            standing_view_height: 26.0,
            crouched: qa_world::movement::q3::types::Q3Posture {
                bounds: Bounds {
                    min: vec3(-15.0, -15.0, -24.0),
                    max: vec3(15.0, 15.0, 16.0),
                },
                view_height: 12.0,
            },
            dead: qa_world::movement::q3::types::Q3Posture {
                bounds: Bounds {
                    min: vec3(-15.0, -15.0, -24.0),
                    max: vec3(15.0, 15.0, -8.0),
                },
                view_height: -8.0,
            },
            invulnerability_expanded: Bounds {
                min: vec3(-40.0, -40.0, -40.0),
                max: vec3(40.0, 40.0, 40.0),
            },
        };
        assert!(fixture.source.fixed_pose(&fixture.player, &postures).unwrap().is_some());
        assert!(fixture.game.has("expand"));
        fixture.client(0).borrow_mut().invulnerability_time = 500;
        assert!(fixture.source.fixed_pose(&fixture.player, &postures).unwrap().is_none());
        assert_eq!(fixture.client(0).borrow().ps.pm_flags & INVULEXPAND, 0);
        let unknown = fixture.host.owner_actor(60);
        let request = damage_request(
            &fixture,
            unknown.id(),
            AttackCause::Environment {
                hazard: qa_content::q3::base::records::EnvironmentHazard::Fall,
            },
        );
        assert!(!fixture.source.blocks_damage(&request).unwrap());
        let request = damage_request(
            &fixture,
            fixture.player.id(),
            AttackCause::Q3 {
                means_of_death: 7,
                damage_flags: 0,
            },
        );
        assert!(!fixture.source.blocks_damage(&request).unwrap());
        assert!(fixture.game.has("blocks"));
    }

    #[test]
    fn grapples_commands_hooks_and_deaths() {
        let fixture = setup();
        fixture.source.admit(&fixture.player).unwrap();
        fixture.client(0).borrow_mut().ps.pm_flags = GRAPPLE_PULL;
        fixture.client(0).borrow_mut().ps.grapple_point = vec3(1.0, 2.0, 3.0);
        assert_eq!(
            fixture.source.grapple_point(fixture.player.id()),
            Some(vec3(1.0, 2.0, 3.0))
        );
        assert!(fixture.source.pull(&fixture.player).is_some());
        fixture.source.command(&fixture.player, false, false, true).unwrap();
        assert!(fixture.game.has("command:1:false:false:true"));
        fixture.game.hook_free.set(true);
        fixture.source.release_hook(fixture.player.id()).unwrap();
        assert!(fixture.game.has("hook_free:1"));
        fixture.source.died(fixture.player.id()).unwrap();
        assert!(fixture.game.has("toss:1"));
        assert!(!fixture.source.gauntlet_hit(&fixture.player).unwrap());
    }

    #[test]
    fn manages_holdables() {
        let fixture = setup();
        fixture.source.admit(&fixture.player).unwrap();
        let items = item_list(Product::Baseq3);
        let index = items
            .iter()
            .position(|item| item.kind.item_type() == ItemType::ItHoldable)
            .unwrap();
        let class = items[index].class_name.unwrap().to_string();
        assert!(fixture.source.give_holdable(&fixture.player, &class).unwrap());
        assert!(!fixture
            .source
            .give_holdable(&fixture.player, "no-such-holdable")
            .unwrap());
        let StatSchema::Base(layout) = stat_schema(Product::Baseq3) else {
            panic!("base schema");
        };
        fixture
            .client(0)
            .borrow_mut()
            .ps
            .stats
            .set(layout.holdable_item as usize, index as i32);
        let tag = item_at(Product::Baseq3, index as i32).unwrap().tag();
        fixture
            .source
            .restore_equipment(
                &fixture.player,
                &Q3SelectedEquipmentState {
                    max_health: 100,
                    persistent_powerup_tag: 0,
                    holdable_item: index as i32,
                    holdable_tag: tag,
                },
            )
            .unwrap();
        fixture.source.consume(&fixture.player, index as i32).unwrap();
        assert_eq!(
            fixture.client(0).borrow().ps.stats.get(layout.holdable_item as usize),
            0
        );
        assert!(matches!(
            fixture.source.consume(&fixture.player, index as i32),
            Err(Q3SourceError::HoldableMismatch)
        ));
    }

    #[test]
    fn captures_and_restores() {
        let first = setup();
        first.source.admit(&first.player).unwrap();
        first.source.fire(&first.player, 2, &step_input(&first.player)).unwrap();
        first.source.synchronize().unwrap();
        assert_eq!(first.host.events.borrow().len(), 1);
        let saved = first.source.capture().unwrap();

        let second = setup();
        second.game.world.borrow_mut().replace(second.world.clone());
        second.game.player.borrow_mut().replace(second.player.clone());
        let actors = second.host.actors.clone();
        let resolve = |saved: SavedActorId| {
            actors
                .owned
                .borrow()
                .values()
                .find(|owned| SavedActorId::from(owned.id()) == saved)
                .cloned()
        };
        let reference = |saved: SavedActorId| {
            resolve(saved)
                .map(|owned| owned.id().clone())
                .unwrap_or_else(|| second.world.id().clone())
        };
        second.source.restore(&saved, &resolve, &reference).unwrap();
        assert_eq!(second.source.generation(), 1);
        assert!(second.source.presentation_baseline().is_some());
        assert!(second.source.live(second.player.id()));
        second.source.synchronize().unwrap();
        assert!(second.host.events.borrow().is_empty());
        assert_eq!(
            second.source.random().borrow().seed(),
            first.source.random().borrow().seed()
        );

        let tampered = {
            let mut saved = saved.clone();
            if let SaveJson::Object(members) = &mut saved {
                for (key, value) in members.iter_mut() {
                    if key == "product" {
                        *value = SaveJson::String("missionpack".to_string());
                    }
                }
            }
            saved
        };
        let third = setup();
        assert!(matches!(
            third.source.restore(&tampered, &resolve, &reference),
            Err(Q3SourceError::World(_))
        ));
        assert!(matches!(
            first.source.restore(&saved, &resolve, &reference),
            Err(Q3SourceError::RestoreRequiresUnused)
        ));
    }

    #[test]
    fn closes_and_cleans_releases() {
        let fixture = setup();
        fixture.source.admit(&fixture.player).unwrap();
        assert!(!fixture.host.strings.borrow().get(544).is_empty());
        fixture.host.actors.release(&fixture.player);
        assert!(!fixture.source.live(fixture.player.id()));
        assert!(fixture.host.strings.borrow().get(544).is_empty());
        fixture.source.close().unwrap();
        assert!(!fixture.source.active());
        assert!(fixture.game.has("close_missiles"));
        fixture.source.close().unwrap();
        assert!(matches!(
            fixture.source.admit(&fixture.player),
            Err(Q3SourceError::Closed)
        ));
        assert!(matches!(fixture.source.synchronize(), Err(Q3SourceError::Closed)));
    }

    #[test]
    fn imports_empty_legacy_save() {
        use super::super::q3_source_legacy::{restore_legacy_q3_source, LegacyQ3Source};
        let fixture = setup();
        let saved = LegacyQ3Source {
            milliseconds: 900,
            random_seed: 77,
            projectiles: Vec::new(),
            statistics: Vec::new(),
            hook_held: Vec::new(),
        };
        restore_legacy_q3_source(&fixture.source, &saved).unwrap();
        assert!(fixture.game.has("import:0"));
        assert_eq!(fixture.source.random().borrow().seed(), 77);
        fixture.source.restore_legacy(&saved, &[]).unwrap();
        assert_eq!(fixture.source.generation(), 1);
    }

    #[test]
    fn rejects_invalid_legacy_saves() {
        use super::super::super::q3_ballistics::Q3ProjectilePhase;
        use super::super::q3_source_legacy::{restore_legacy_q3_source, LegacyQ3Source};
        use qa_content::q3::base::game::projectile::Q3Projectile;
        let fixture = setup();
        let trajectory = Trajectory {
            trajectory_type: TrajectoryType::TrStationary,
            time: 0,
            duration: 0,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(0.0, 0.0, 0.0),
        };
        let stranger = fixture.host.owner_actor(70);
        let ghost = fixture.host.actors.owner.actor(999, 1);
        let save_with = |state: Q3ProjectileState| LegacyQ3Source {
            milliseconds: 900,
            random_seed: 77,
            projectiles: vec![state],
            statistics: Vec::new(),
            hook_held: Vec::new(),
        };
        let ghost_state = Q3ProjectileState {
            base: Q3Projectile {
                actor: ghost.clone(),
                owner: fixture.player.id().clone(),
                weapon: 5,
                direct: 10,
                splash: 10,
                radius: 60,
                method: 1,
                splash_method: 1,
                damage_point: vec3(0.0, 0.0, 0.0),
                trajectory,
                flags: 0,
                pass: None,
            },
            expires: 0,
            phase: Q3ProjectilePhase::Flight,
        };
        assert!(matches!(
            restore_legacy_q3_source(&fixture.source, &save_with(ghost_state)),
            Err(Q3SourceError::LegacyProjectileBody)
        ));
        fixture
            .host
            .bodies
            .states
            .borrow_mut()
            .insert(stranger.id().clone(), body_at(vec3(0.0, 0.0, 0.0)));
        let attached = Q3ProjectileState {
            base: Q3Projectile {
                actor: stranger.id().clone(),
                owner: fixture.player.id().clone(),
                weapon: 5,
                direct: 10,
                splash: 10,
                radius: 60,
                method: 1,
                splash_method: 1,
                damage_point: vec3(0.0, 0.0, 0.0),
                trajectory,
                flags: 0,
                pass: None,
            },
            expires: 0,
            phase: Q3ProjectilePhase::Attached {
                target: None,
                next_think: 0,
            },
        };
        assert!(matches!(
            restore_legacy_q3_source(&fixture.source, &save_with(attached)),
            Err(Q3SourceError::LegacyAttachedHook)
        ));
    }

    impl MockHost {
        fn owner_actor(&self, slot: u32) -> OwnedActor {
            self.actors.allocate(&self.provider, slot, "q3:test")
        }
    }

    fn step_input(player: &OwnedActor) -> WeaponStepInput {
        WeaponStepInput {
            actor: player.clone(),
            command: qa_world::movement::types::UserCommand::Q3(qa_world::movement::types::Q3UserCommand {
                server_time_milliseconds: 1000,
                angle_words: [0, 0, 0],
                buttons: 0,
                weapon: 0,
                forward_move: 0,
                right_move: 0,
                up_move: 0,
            }),
            frame: qa_core::time::FrameContext {
                frame: 1,
                phase: qa_core::time::FramePhase::ClientCommand,
                time: qa_core::time::SourceTime::Milliseconds(1000),
                elapsed: qa_core::time::SourceTime::Milliseconds(100),
            },
            arsenal: qa_world::movement::types::ArsenalState {
                provider: ProviderId::new("q3", "test"),
                active_weapon: None,
                ammo: Vec::new(),
                state: qa_world::movement::types::WeaponState::Q3 {
                    source_weapon: 2,
                    state: 0,
                    time_milliseconds: 0,
                },
            },
            animation: qa_world::movement::types::ActorAnimationState {
                provider: ProviderId::new("q3", "test"),
                state: qa_world::movement::types::AnimationState::Q3 {
                    legs: 0,
                    torso: 0,
                    legs_timer_milliseconds: 0,
                    torso_timer_milliseconds: 0,
                },
            },
            environment: qa_world::movement::types::MovementEnvironment::default(),
            gauntlet_hit: false,
        }
    }

    fn damage_request(_fixture: &Fixture, target: &ActorId, cause: AttackCause) -> DamageRequest {
        DamageRequest {
            attack: qa_content::q3::base::records::AttackProvenance {
                sequence: 1,
                time: qa_core::time::SourceTime::Milliseconds(1000),
                attacker: None,
                inflictor: None,
                originating_projectile: None,
                weapon: None,
                weapon_provider: ProviderId::new("q3", "test"),
                damage_powerup_owner: None,
                combat_provider: ProviderId::new("sim", "combat"),
                inventory_provider: ProviderId::new("sim", "inventory"),
                movement_provider: ProviderId::new("sim", "movement"),
                cause,
            },
            target: target.clone(),
            amount: 10.0,
            knockback: 0.0,
            direction: vec3(0.0, 0.0, 1.0),
            point: vec3(0.0, 0.0, 0.0),
            normal: vec3(0.0, 0.0, 1.0),
            delivery: qa_world::combat::Delivery::Direct,
        }
    }
}
