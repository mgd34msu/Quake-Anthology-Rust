//! Q2-local gameplay/world/scene contract types.
//!
//! Donor provenance (shapes only; owned by other lanes):
//! `src/contracts/gameplay.ts` (`AttackProvenance`, `DamageRequest`,
//! `DamageDecision`, `DamageOutcome`, `CombatState`, `SourceDamageModifier`,
//! `Q2NativeCause`, `TransitionIntent`), `src/contracts/world.ts`
//! (`BodyState`, `BodyAttachment`, `LinkedBody`, `TouchContact`,
//! `PainReaction`, `DeathReaction`), `src/contracts/scene.ts`
//! (`TraceResult` and parts), `src/contracts/flare.ts` (`SceneFlare`),
//! `src/contracts/weapon-behavior.ts` (`WeaponTrajectoryUpdate`,
//! `WeaponBehaviorProjectilePort`), `src/world/gameplay/authority.ts`
//! (`CombatTraits`, `PowerArmorCellBinding`),
//! `src/world/gameplay/policies.ts` (`Q2DamageSourceEffects`),
//! `src/contracts/equipment.ts` (`SharedGrappleControl`),
//! `src/contracts/content.ts` (`GrappleSelection` and parts).
//!
//! Armor, inventory-entry, pickup, held-weapon, model-attachment and
//! identity/math types are reused from shared modules, never redefined.

use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::math::{Bounds, Plane, Vec3};
use qa_core::time::SourceTime;

use crate::contract::{ArmorState, ItemId, ProjectileRole};

/// Original mod damage-cause encodings (`Q2NativeCause`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2NativeCause {
    /// Classic game DLL cause ordinal.
    Classic {
        /// Original game.
        game: Q2NativeGame,
        /// Native ordinal.
        value: i32,
    },
    /// Rerelease cause id.
    Rerelease {
        /// Cause id.
        id: i32,
        /// Friendly fire flag.
        friendly_fire: bool,
        /// No point loss flag.
        no_point_loss: bool,
    },
}

/// Classic game carrying a native cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2NativeGame {
    /// Base game.
    Base,
    /// Xatrix mission pack.
    Xatrix,
    /// Rogue mission pack.
    Rogue,
    /// Capture the flag.
    Ctf,
}

/// Damage cause carried by an attack (`AttackProvenance["cause"]`).
#[derive(Debug, Clone, PartialEq)]
pub enum AttackCause {
    /// Quake cause.
    Q1 {
        /// Death type.
        death_type: String,
        /// Armor effect override.
        armor_effect: Option<Q1ArmorEffect>,
    },
    /// Quake II cause. `means_of_death` is the canonical engine cause id,
    /// never an unconverted native ordinal.
    Q2 {
        /// Canonical cause id.
        means_of_death: i32,
        /// Damage flags.
        damage_flags: i32,
        /// Native cause, when preserved.
        native: Option<Q2NativeCause>,
    },
    /// Quake III cause.
    Q3 {
        /// Canonical cause id.
        means_of_death: i32,
        /// Damage flags.
        damage_flags: i32,
    },
    /// Environmental hazard.
    Environment {
        /// Hazard kind.
        hazard: EnvironmentHazard,
    },
}

/// Quake armor effect override on a Q1 cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1ArmorEffect {
    /// Bypass armor.
    Bypass,
    /// Half effectiveness.
    HalfEffectiveness,
}

/// Environmental hazard kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnvironmentHazard {
    /// Falling.
    Fall,
    /// Drowning.
    Drown,
    /// Lava.
    Lava,
    /// Slime.
    Slime,
    /// Crushing.
    Crush,
    /// Trigger hurt.
    Trigger,
}

/// Captured before any combat mutation (`AttackProvenance`).
#[derive(Debug, Clone, PartialEq)]
pub struct AttackProvenance {
    /// Source sequence number.
    pub sequence: u64,
    /// Source time of the attack.
    pub time: SourceTime,
    /// Attacking actor.
    pub attacker: Option<ActorId>,
    /// Inflicting actor.
    pub inflictor: Option<ActorId>,
    /// Originating projectile, when routed through one.
    pub originating_projectile: Option<ActorId>,
    /// Attacking weapon item.
    pub weapon: Option<ItemId>,
    /// Weapon provider.
    pub weapon_provider: ProviderId,
    /// This source already applied its damage modifier.
    pub damage_powerup_owner: Option<ProviderId>,
    /// Combat provider.
    pub combat_provider: ProviderId,
    /// Inventory provider.
    pub inventory_provider: ProviderId,
    /// Movement provider.
    pub movement_provider: ProviderId,
    /// Damage cause.
    pub cause: AttackCause,
}

/// Original attacker damage policy evaluated at hit time
/// (`SourceDamageModifier`).
#[derive(Debug, Clone)]
pub struct SourceDamageModifier {
    /// Owning provider.
    pub owner: ProviderId,
    /// Transform the source amount for an attacker.
    pub transform: fn(Option<&ActorId>, f64) -> f64,
}

/// Damage request delivered to combat (`DamageRequest`).
#[derive(Debug, Clone, PartialEq)]
pub struct DamageRequest {
    /// Attack provenance.
    pub attack: AttackProvenance,
    /// Target actor.
    pub target: ActorId,
    /// Damage amount.
    pub amount: f64,
    /// Knockback amount.
    pub knockback: f64,
    /// Damage direction.
    pub direction: Vec3,
    /// Damage point.
    pub point: Vec3,
    /// Surface normal.
    pub normal: Vec3,
    /// Delivery kind.
    pub delivery: DamageDelivery,
}

/// Damage delivery kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DamageDelivery {
    /// Direct damage.
    Direct,
    /// Radius damage.
    Radius,
}

/// One committed combat mutation (`DamageMutation`).
#[derive(Debug, Clone, PartialEq)]
pub enum DamageMutation {
    /// Health change.
    Health {
        /// Health before.
        before: f64,
        /// Health after.
        after: f64,
    },
    /// Armor change.
    Armor {
        /// Armor before.
        before: ArmorState,
        /// Armor after.
        after: ArmorState,
    },
    /// Source velocity change.
    SourceVelocity {
        /// Velocity before.
        before: Vec3,
        /// Velocity after.
        after: Vec3,
        /// Movement provider.
        movement_provider: ProviderId,
    },
    /// Knockback impulse.
    Impulse {
        /// Impulse vector.
        impulse: Vec3,
        /// Movement provider.
        movement_provider: ProviderId,
    },
}

/// Committed damage decision (`DamageDecision`).
#[derive(Debug, Clone, PartialEq)]
pub struct DamageDecision {
    /// Original request.
    pub request: DamageRequest,
    /// Committed mutations.
    pub mutations: Vec<DamageMutation>,
    /// Damage applied to health.
    pub applied_damage: f64,
    /// Target reaction.
    pub reaction: DamageReactionKind,
    /// Source savings feedback.
    pub feedback: Option<DamageFeedback>,
}

/// Damage reaction kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DamageReactionKind {
    /// No reaction.
    None,
    /// Pain reaction.
    Pain,
    /// Death reaction.
    Death,
}

/// Source savings feedback (`DamageDecision["feedback"]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DamageFeedback {
    /// Quake II savings.
    Q2 {
        /// Power armor saved.
        power_armor: f64,
        /// Armor saved.
        armor: f64,
        /// Blood drawn.
        blood: f64,
        /// Knockback applied.
        knockback: f64,
    },
    /// Quake III savings.
    Q3 {
        /// Knockback applied.
        knockback: f64,
        /// Battlesuit absorbed.
        battlesuit: bool,
    },
}

/// Synchronous damage outcome (`DamageOutcome`).
#[derive(Debug, Clone, PartialEq)]
pub enum DamageOutcome {
    /// Target lifetime ended before the decision committed.
    StaleTarget {
        /// Original request.
        request: DamageRequest,
    },
    /// Decision committed.
    Committed {
        /// Committed decision.
        decision: DamageDecision,
        /// Target survived.
        survived: bool,
    },
}

/// Combatant state (`CombatState`).
#[derive(Debug, Clone, PartialEq)]
pub struct CombatState {
    /// Health.
    pub health: f64,
    /// Armor.
    pub armor: ArmorState,
    /// Mass.
    pub mass: f64,
    /// Whether the actor can take damage.
    pub can_take_damage: bool,
    /// Whether the actor is invulnerable.
    pub invulnerable: bool,
    /// Source entity immunity to damage momentum.
    pub no_knockback: bool,
    /// Team name.
    pub team: Option<String>,
}

/// Mutable combat traits (`CombatTraits`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CombatTraitChanges {
    /// New damageable flag.
    pub can_take_damage: Option<bool>,
    /// New mass.
    pub mass: Option<f64>,
    /// New invulnerability.
    pub invulnerable: Option<bool>,
    /// New team.
    pub team: Option<Option<String>>,
    /// New knockback immunity.
    pub no_knockback: Option<bool>,
}

/// Power armor cell storage owned by the source
/// (`PowerArmorCellBinding`).
pub trait PowerArmorCells {
    /// Read the stored cell count.
    fn read(&self) -> f64;
    /// Write the stored cell count.
    fn write(&mut self, count: f64);
}

/// Physical body state (`BodyState`).
#[derive(Debug, Clone, PartialEq)]
pub struct BodyState {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Collision bounds.
    pub bounds: Bounds,
    /// Ground actor.
    pub ground: Option<ActorId>,
}

/// Attached body follow mode (`BodyAttachment["follow"]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BodyFollow {
    /// Follow anchor translation with an offset.
    Translation {
        /// Offset from the anchor.
        offset: Vec3,
    },
    /// Follow the anchor center.
    Center,
    /// Follow the anchor bounds minimum with an offset.
    BoundsMin {
        /// Offset from the bounds minimum.
        offset: Vec3,
    },
}

/// Attached body record (`BodyAttachment`).
#[derive(Debug, Clone, PartialEq)]
pub struct BodyAttachment {
    /// Anchor actor.
    pub anchor: ActorId,
    /// Follow mode.
    pub follow: BodyFollow,
}

/// Linked body snapshot (`LinkedBody`).
#[derive(Debug, Clone, PartialEq)]
pub struct LinkedBody {
    /// Linked actor.
    pub actor: ActorId,
    /// Body state at link time.
    pub state: BodyState,
    /// Absolute bounds captured by the link.
    pub absolute_bounds: Bounds,
    /// Link count.
    pub link_count: u64,
}

/// Touch surface description (`TouchContact["surface"]`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TouchSurface {
    /// Surface name.
    pub name: String,
    /// Native flags.
    pub native_flags: i32,
    /// Native value.
    pub native_value: i32,
}

/// Rerelease touch source trace (`TouchContact["sourceTrace"]`).
#[derive(Debug, Clone, PartialEq)]
pub struct TouchSourceTrace {
    /// Traced actor.
    pub ent: ActorId,
    /// Whether the trace was inverted.
    pub inverted: bool,
    /// Q2 trace fields.
    pub trace: Q2TraceFields,
}

/// Touch contact (`TouchContact`).
#[derive(Debug, Clone, PartialEq)]
pub struct TouchContact {
    /// Touched actor.
    pub this: OwnedActor,
    /// Other actor.
    pub other: ActorId,
    /// Contact plane.
    pub plane: Option<Plane>,
    /// Contact surface.
    pub surface: Option<TouchSurface>,
    /// Rerelease source trace.
    pub source_trace: Option<TouchSourceTrace>,
}

/// Pain reaction (`PainReaction`).
#[derive(Debug, Clone, PartialEq)]
pub struct PainReaction {
    /// Attack provenance.
    pub attack: Option<AttackProvenance>,
    /// Reacting actor.
    pub this: OwnedActor,
    /// Attacking actor.
    pub attacker: Option<ActorId>,
    /// Knockback kick.
    pub kick: f64,
    /// Damage taken.
    pub damage: f64,
}

/// Death reaction (`DeathReaction`).
#[derive(Debug, Clone, PartialEq)]
pub struct DeathReaction {
    /// Pain fields.
    pub pain: PainReaction,
    /// Inflicting actor.
    pub inflictor: Option<ActorId>,
    /// Death point.
    pub point: Vec3,
}

impl DeathReaction {
    /// Attack provenance.
    #[must_use]
    pub fn attack(&self) -> Option<&AttackProvenance> {
        self.pain.attack.as_ref()
    }

    /// Reacting actor.
    #[must_use]
    pub fn this(&self) -> &OwnedActor {
        &self.pain.this
    }

    /// Attacking actor.
    #[must_use]
    pub fn attacker(&self) -> Option<&ActorId> {
        self.pain.attacker.as_ref()
    }

    /// Knockback kick.
    #[must_use]
    pub fn kick(&self) -> f64 {
        self.pain.kick
    }

    /// Damage taken.
    #[must_use]
    pub fn damage(&self) -> f64 {
        self.pain.damage
    }
}

/// Trace hit record (`TraceHit`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TraceHit {
    /// No hit.
    None,
    /// World hit.
    World {
        /// Brush model number.
        model: i32,
    },
    /// Actor hit.
    Actor {
        /// Hit actor.
        actor: ActorId,
    },
}

/// Trace contact record (`TraceContact`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TraceContact {
    /// No contact.
    None,
    /// Plane contact.
    Plane {
        /// Contact plane.
        plane: Plane,
    },
}

/// Quake II surface info (`Q2SurfaceInfo`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Q2SurfaceInfo {
    /// Surface name.
    pub name: String,
    /// Surface flags.
    pub flags: i32,
    /// Surface value.
    pub value: i32,
    /// Surface material.
    pub material: String,
}

/// Quake II BSP plane (`BspPlane`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2BspPlane {
    /// Plane normal.
    pub normal: Vec3,
    /// Plane distance.
    pub distance: f32,
    /// Plane type.
    pub plane_type: i32,
    /// Sign bits.
    pub signbits: i32,
}

/// Quake II trace fields (`TraceResult & { kind: "q2" }`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2TraceFields {
    /// Contents flags.
    pub contents: i32,
    /// Hit surface.
    pub surface: Option<Q2SurfaceInfo>,
    /// Source plane.
    pub source_plane: Q2BspPlane,
    /// Secondary plane and surface.
    pub secondary: Option<Q2SecondaryPlane>,
}

/// Quake II secondary trace plane.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2SecondaryPlane {
    /// Secondary plane.
    pub plane: Q2BspPlane,
    /// Secondary surface.
    pub surface: Option<Q2SurfaceInfo>,
}

/// Trace result (`TraceResult`). Only Q2 traces occur in Q2 content, but
/// the kind tag is preserved so engine traces stay total.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceResult {
    /// Fraction of the trace completed.
    pub fraction: f64,
    /// Trace end point.
    pub end: Vec3,
    /// Trace started inside solid.
    pub start_solid: bool,
    /// Trace was entirely inside solid.
    pub all_solid: bool,
    /// Contact record.
    pub contact: TraceContact,
    /// Hit record.
    pub hit: TraceHit,
    /// Family-specific fields.
    pub family: TraceFamily,
}

/// Family-specific trace fields.
#[derive(Debug, Clone, PartialEq)]
pub enum TraceFamily {
    /// Quake fields.
    Q1 {
        /// Started in open space.
        in_open: bool,
        /// Started in water.
        in_water: bool,
        /// Source plane.
        source_plane: Plane,
        /// Surface flags.
        surface_flags: Option<i32>,
    },
    /// Quake II fields.
    Q2(Q2TraceFields),
    /// Quake III fields.
    Q3 {
        /// Contents flags.
        contents: i32,
        /// Surface flags.
        surface_flags: i32,
        /// Source plane.
        source_plane: Q2BspPlane,
    },
}

impl TraceResult {
    /// Q2 family fields, when this is a Q2 trace.
    #[must_use]
    pub fn q2(&self) -> Option<&Q2TraceFields> {
        match &self.family {
            TraceFamily::Q2(fields) => Some(fields),
            _ => None,
        }
    }
}

/// Scene flare record (`SceneFlare`).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneFlare {
    /// Flare image path.
    pub image: String,
    /// Fade start distance.
    pub fade_start: f64,
    /// Fade end distance.
    pub fade_end: f64,
    /// Flare scale.
    pub scale: f64,
    /// Flare color bytes.
    pub color: Vec3,
    /// Rim color bytes.
    pub rim_color: Option<Vec3>,
    /// Whether the flare locks its angle.
    pub lock_angle: bool,
}

/// Weapon trajectory update (`WeaponTrajectoryUpdate`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WeaponTrajectoryUpdate {
    /// New origin.
    pub origin: Vec3,
    /// New velocity.
    pub velocity: Vec3,
    /// New angles.
    pub angles: Vec3,
}

/// Selected projectile-trajectory port (`WeaponBehaviorProjectilePort`).
pub trait WeaponBehaviorProjectilePort {
    /// Whether the port controls a projectile's trajectory.
    fn controls_trajectory(&self, projectile: &ActorId) -> bool;
    /// Launch a projectile.
    fn launch(&mut self, input: &WeaponBehaviorLaunch) -> Option<WeaponTrajectoryUpdate>;
    /// Step a projectile trajectory.
    fn step(
        &mut self,
        projectile: &OwnedActor,
        body: &BodyState,
        time_seconds: f64,
    ) -> Option<WeaponTrajectoryUpdate>;
}

/// Weapon behavior launch input (`WeaponBehaviorLaunch`).
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponBehaviorLaunch {
    /// Projectile actor.
    pub projectile: OwnedActor,
    /// Shooter actor.
    pub shooter: ActorId,
    /// Weapon item.
    pub weapon: ItemId,
    /// Projectile role.
    pub role: ProjectileRole,
    /// Launch time in seconds.
    pub time_seconds: f64,
    /// Launch body.
    pub body: BodyState,
}

/// Current combatant states for source effects (`CurrentCombatState`).
pub trait CurrentCombatState {
    /// Current target state.
    fn target(&self) -> Option<CombatState>;
    /// Current attacker state.
    fn attacker(&self) -> Option<CombatState>;
}

/// Quake II damage source effects (`Q2DamageSourceEffects`).
///
/// Every hook is optional in the donor; the defaults below are the
/// identity effects.
pub trait Q2DamageSourceEffects {
    /// Adjust damage before momentum (CTF strength, LMCTF damage rune).
    fn before_momentum(
        &mut self,
        request: &DamageRequest,
        damage: f64,
        target: &CombatState,
        attacker: Option<&CombatState>,
    ) -> f64 {
        let _ = (request, target, attacker);
        damage
    }

    /// Whether power armor applies.
    fn power_armor_allowed(
        &mut self,
        request: &DamageRequest,
        target: &CombatState,
        attacker: Option<&CombatState>,
    ) -> bool {
        let _ = (request, target, attacker);
        true
    }

    /// Adjust damage after power armor (LMCTF resistance).
    fn after_power_armor(
        &mut self,
        request: &DamageRequest,
        take: f64,
        target: &CombatState,
        attacker: Option<&CombatState>,
    ) -> f64 {
        let _ = (request, target, attacker);
        take
    }

    /// Whether regular armor applies.
    fn armor_allowed(
        &mut self,
        request: &DamageRequest,
        target: &CombatState,
        attacker: Option<&CombatState>,
    ) -> bool {
        let _ = (request, target, attacker);
        true
    }

    /// Adjust damage after armor (CTF resistance).
    fn after_armor(
        &mut self,
        request: &DamageRequest,
        take: f64,
        target: &CombatState,
        attacker: Option<&CombatState>,
    ) -> f64 {
        let _ = (request, target, attacker);
        take
    }

    /// Observe committed victim health before death is tested (LMCTF
    /// vampire healing).
    fn after_health(&mut self, decision: &DamageDecision, current: &dyn CurrentCombatState) {
        let _ = (decision, current);
    }
}

/// Mission gate carried by a transition intent.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MissionGate {
    /// Objective id.
    pub objective: String,
    /// Whether the gate is satisfied.
    pub satisfied: bool,
}

/// Level/match transition intent (`TransitionIntent`).
#[derive(Debug, Clone, PartialEq)]
pub enum TransitionIntent {
    /// Travel to a campaign level.
    CampaignLevel {
        /// Campaign provider.
        campaign: ProviderId,
        /// Destination map.
        map: String,
        /// Spawn point.
        spawn_point: String,
        /// Mission gates.
        gates: Vec<MissionGate>,
        /// Causing actor.
        cause: Option<ActorId>,
    },
    /// Campaign completed.
    CampaignComplete {
        /// Campaign provider.
        campaign: ProviderId,
        /// Mission gates.
        gates: Vec<MissionGate>,
    },
    /// Competitive round completed.
    RoundComplete {
        /// Match provider.
        provider: ProviderId,
        /// Winner name.
        winner: Option<String>,
    },
    /// Match map rotation.
    MatchRotation {
        /// Match provider.
        provider: ProviderId,
        /// Destination map.
        map: String,
    },
}

impl TransitionIntent {
    /// Travel to a campaign level.
    pub fn campaign_level(
        campaign: ProviderId,
        map: String,
        spawn_point: String,
        gates: Vec<MissionGate>,
        cause: Option<ActorId>,
    ) -> Self {
        TransitionIntent::CampaignLevel {
            campaign,
            map,
            spawn_point,
            gates,
            cause,
        }
    }
}

/// Live actor observation (`ActorObservation`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActorObservation {
    /// Observed actor.
    pub id: ActorId,
    /// Owning provider.
    pub owner: ProviderId,
    /// Source definition text.
    pub definition: String,
}

/// Grapple provider reference (`ProviderReference`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GrappleProviderReference {
    /// Owning provider.
    pub provider: ProviderId,
    /// Content id.
    pub content: String,
}

/// Grapple binding (`GrappleSelection["binding"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GrappleBinding {
    /// Weapon slot.
    Slot,
    /// Offhand.
    Offhand,
}

/// Grapple mechanic (`GrappleSelection["mechanic"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GrappleMechanic {
    /// Q1 threewave.
    Q1Threewave,
    /// Q2 CTF.
    Q2Ctf,
    /// Q2 LMCTF.
    Q2Lmctf,
    /// Q3 QVM.
    Q3Qvm,
}

/// Grapple edition (`GrappleSelection["edition"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GrappleEdition {
    /// Classic.
    Classic,
    /// Rerelease.
    Rerelease,
}

/// QVM grapple profile reference (`QvmGrappleDefinition` projection).
///
/// Q2 content only routes on the selection; the executable-bound definition
/// stays Q3 execution scope, so the shim keeps the profile identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmGrappleReference {
    /// Profile id.
    pub id: String,
    /// Profile title.
    pub title: String,
}

/// Grapple selection (`GrappleSelection`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GrappleSelection {
    /// Disabled.
    Disabled,
    /// Enabled.
    Enabled {
        /// Source.
        source: GrappleProviderReference,
        /// Binding.
        binding: GrappleBinding,
        /// Mechanic.
        mechanic: GrappleMechanic,
        /// Edition.
        edition: GrappleEdition,
        /// QVM profile; present only for the Q3 mechanic.
        profile: Option<QvmGrappleReference>,
    },
}

/// Shared grapple control (`SharedGrappleControl`).
pub trait SharedGrappleControl: std::fmt::Debug {
    /// Read the selection.
    fn selection(&self) -> &GrappleSelection;
    /// Whether the mechanic owns the native slot.
    fn native_slot(&self, mechanic: GrappleMechanic) -> bool;
    /// Feed offhand input.
    fn input(&mut self, actor: &ActorId, held: bool);
    /// Release the actor.
    fn release(&mut self, actor: &ActorId);
    /// Whether the actor is pulling.
    fn pulling(&self, actor: &ActorId) -> bool;
    /// Gravity scale.
    fn gravity_scale(&self, actor: &ActorId) -> u8;
}
