//! Shared gameplay/world structural types used by the Q1 QuakeC bindings.
//!
//! Donor provenance: `src/contracts/gameplay.ts` (`AttackProvenance`,
//! `DamageRequest`, `DamageOutcome`, `DamageDecision`, `DamageMutation`,
//! `ArmorDamageFlags`, `ArmorStageInput`), `src/world/gameplay/armor.ts`
//! (`attackDamageFlags`), `src/world/gameplay/authority.ts`
//! (`SourceDamageResult`, `SourceDamageObserver`), and the used
//! `world/actors` registry/slot surface (`SessionActorRegistry`,
//! `SourceActorSlots`).
//!
//! This file imports nothing from its siblings so it can be deleted
//! once `crate::q1::foundation::gameplay` lands. Armor storage types
//! reuse [`crate::contract`]. Behavioral traits that need [`QcError`](super::QcError)
//! (`GameplayAuthority`, `SourceArmorStage`) live in [`super::qc_view`].

use qa_core::identity::{ActorId, OwnedActor, ProviderId, SavedActorId};
use qa_core::math::Vec3;
use qa_core::time::SourceTime;

use crate::contract::{ArmorState, ItemId};

/// Q1 armor effect carried by a death type (donor `cause.armorEffect`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1ArmorEffect {
    /// Bypass armor.
    Bypass,
    /// Halve armor effectiveness.
    HalfEffectiveness,
}

/// Environmental hazard (donor `cause.hazard`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnvHazard {
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
    /// Trigger.
    Trigger,
}

/// Attack cause (donor `AttackProvenance.cause`).
#[derive(Debug, Clone, PartialEq)]
pub enum DamageCause {
    /// Quake 1 death type.
    Q1 {
        /// Death type name.
        death_type: String,
        /// Armor effect override.
        armor_effect: Option<Q1ArmorEffect>,
    },
    /// Quake II canonical cause.
    Q2 {
        /// Canonical means of death.
        means_of_death: i32,
        /// Damage flags.
        damage_flags: i32,
    },
    /// Quake III canonical cause.
    Q3 {
        /// Canonical means of death.
        means_of_death: i32,
        /// Damage flags.
        damage_flags: i32,
    },
    /// Environmental hazard.
    Environment {
        /// Hazard kind.
        hazard: EnvHazard,
    },
}

/// Captured attack provenance (donor `AttackProvenance`).
#[derive(Debug, Clone, PartialEq)]
pub struct AttackProvenance {
    /// Sequence number.
    pub sequence: u64,
    /// Source time.
    pub time: SourceTime,
    /// Attacking actor.
    pub attacker: Option<ActorId>,
    /// Inflicting actor.
    pub inflictor: Option<ActorId>,
    /// Originating projectile.
    pub originating_projectile: Option<ActorId>,
    /// Attack weapon.
    pub weapon: Option<ItemId>,
    /// Weapon provider.
    pub weapon_provider: ProviderId,
    /// Provider that already applied its damage modifier.
    pub damage_powerup_owner: Option<ProviderId>,
    /// Combat provider.
    pub combat_provider: ProviderId,
    /// Inventory provider.
    pub inventory_provider: ProviderId,
    /// Movement provider.
    pub movement_provider: ProviderId,
    /// Attack cause.
    pub cause: DamageCause,
}

/// Damage delivery (donor `DamageRequest.delivery`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DamageDelivery {
    /// Direct damage.
    Direct,
    /// Radius damage.
    Radius,
}

/// Damage request (donor `DamageRequest`).
#[derive(Debug, Clone, PartialEq)]
pub struct DamageRequest {
    /// Attack provenance.
    pub attack: AttackProvenance,
    /// Target actor.
    pub target: ActorId,
    /// Damage amount.
    pub amount: f64,
    /// Knockback.
    pub knockback: f64,
    /// Hit direction.
    pub direction: Vec3,
    /// Hit point.
    pub point: Vec3,
    /// Hit normal.
    pub normal: Vec3,
    /// Delivery.
    pub delivery: DamageDelivery,
}

/// Damage reaction (donor `reaction`: `none` | `pain` | `death`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DamageReaction {
    /// No reaction.
    None,
    /// Pain reaction.
    Pain,
    /// Death reaction.
    Death,
}

/// Validated source damage result (donor `SourceDamageResult`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceDamageResult {
    /// Applied damage.
    pub applied_damage: f64,
    /// Reaction.
    pub reaction: DamageReaction,
}

/// Committed damage mutation (donor `DamageMutation`).
#[derive(Debug, Clone, PartialEq)]
pub enum DamageMutation {
    /// Health store.
    Health {
        /// Health before.
        before: f64,
        /// Health after.
        after: f64,
    },
    /// Armor store.
    Armor {
        /// Armor before.
        before: ArmorState,
        /// Armor after.
        after: ArmorState,
    },
    /// Source velocity store.
    SourceVelocity {
        /// Velocity before.
        before: Vec3,
        /// Velocity after.
        after: Vec3,
        /// Movement provider.
        movement_provider: ProviderId,
    },
    /// Applied impulse.
    Impulse {
        /// Impulse vector.
        impulse: Vec3,
        /// Movement provider.
        movement_provider: ProviderId,
    },
}

/// Source-observed store (donor `Exclude<DamageMutation, { kind:
/// "impulse" }>`).
#[derive(Debug, Clone, PartialEq)]
pub enum SourceStoredMutation {
    /// Health store.
    Health {
        /// Health before.
        before: f64,
        /// Health after.
        after: f64,
    },
    /// Armor store.
    Armor {
        /// Armor before.
        before: ArmorState,
        /// Armor after.
        after: ArmorState,
    },
    /// Source velocity store.
    SourceVelocity {
        /// Velocity before.
        before: Vec3,
        /// Velocity after.
        after: Vec3,
        /// Movement provider.
        movement_provider: ProviderId,
    },
}

/// Committed damage decision (donor `DamageDecision`).
#[derive(Debug, Clone, PartialEq)]
pub struct DamageDecision {
    /// Damage request.
    pub request: DamageRequest,
    /// Committed mutations.
    pub mutations: Vec<DamageMutation>,
    /// Applied damage.
    pub applied_damage: f64,
    /// Reaction.
    pub reaction: DamageReaction,
}

/// Damage outcome (donor `DamageOutcome`).
#[derive(Debug, Clone, PartialEq)]
pub enum DamageOutcome {
    /// Stale target.
    StaleTarget {
        /// Damage request.
        request: DamageRequest,
    },
    /// Committed decision.
    Committed {
        /// Decision.
        decision: DamageDecision,
        /// Whether the target survived.
        survived: bool,
    },
}

/// Observes validated source damage operations (donor
/// `SourceDamageObserver`). Protocol violations panic on the authority
/// side; reachable binding flows always succeed.
pub trait SourceDamageObserver {
    /// Observe one committed store.
    fn stored(&self, write: SourceStoredMutation);
    /// Observe the reaction boundary exactly once.
    fn before_reaction(&self, result: &SourceDamageResult);
}

/// Armor stage under evaluation (donor `ArmorDamageFlags.stage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArmorStageKind {
    /// Power stage.
    Power,
    /// Regular stage.
    Regular,
}

/// Armor damage flags (donor `ArmorDamageFlags`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArmorDamageFlags {
    /// Stage under evaluation.
    pub stage: Option<ArmorStageKind>,
    /// Bypass all armor.
    pub no_armor: bool,
    /// Bypass power armor.
    pub no_power_armor: bool,
    /// Bypass regular armor.
    pub no_regular_armor: bool,
    /// Energy damage.
    pub energy: bool,
    /// Regular protection scale.
    pub regular_protection_scale: Option<f64>,
}

/// Armor stage hit geometry (donor `ArmorStageInput.geometry`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DamageGeometry {
    /// Hit direction.
    pub direction: Vec3,
    /// Hit point.
    pub point: Vec3,
    /// Hit normal.
    pub normal: Vec3,
}

/// Armor stage input (donor `ArmorStageInput`).
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorStageInput {
    /// Damage request.
    pub request: DamageRequest,
    /// Hit geometry.
    pub geometry: DamageGeometry,
    /// Amount entering the stage.
    pub amount: f64,
    /// Damage flags.
    pub flags: ArmorDamageFlags,
}

/// Decoded attack flags (donor `attackDamageFlags`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttackDamageFlags {
    /// Armor flags.
    pub armor: ArmorDamageFlags,
    /// Immunity to damage momentum.
    pub no_knockback: bool,
    /// No protection applies.
    pub no_protection: bool,
    /// No team protection applies.
    pub no_team_protection: bool,
    /// Destroy armor.
    pub destroy_armor: bool,
}

/// Decode native flags according to their origin, never reinterpreted
/// as another game's bit positions (donor `attackDamageFlags` from
/// `src/world/gameplay/armor.ts`).
#[must_use]
pub fn attack_damage_flags(request: &DamageRequest) -> AttackDamageFlags {
    let cause = &request.attack.cause;
    let q2 = match cause {
        DamageCause::Q2 { damage_flags, .. } => *damage_flags,
        _ => 0,
    };
    let q3 = match cause {
        DamageCause::Q3 { damage_flags, .. } => *damage_flags,
        _ => 0,
    };
    AttackDamageFlags {
        armor: ArmorDamageFlags {
            stage: None,
            no_armor: (q2 | q3) & 2 != 0
                || matches!(
                    cause,
                    DamageCause::Q1 { armor_effect, .. }
                        if *armor_effect == Some(Q1ArmorEffect::Bypass)
                ),
            no_power_armor: q2 & 0x100 != 0,
            no_regular_armor: q2 & 0x80 != 0,
            energy: q2 & 4 != 0,
            regular_protection_scale: Some(
                if matches!(
                    cause,
                    DamageCause::Q1 { armor_effect, .. }
                        if *armor_effect == Some(Q1ArmorEffect::HalfEffectiveness)
                ) {
                    0.5
                } else {
                    1.0
                },
            ),
        },
        no_knockback: q2 & 8 != 0 || q3 & 4 != 0,
        no_protection: q2 & 0x20 != 0 || q3 & 8 != 0,
        no_team_protection: q3 & 0x10 != 0,
        destroy_armor: q2 & 0x40 != 0,
    }
}

/// Live actor source projection (donor `SessionActorRegistry.sourceOf`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorSource {
    /// Owning provider.
    pub provider: ProviderId,
    /// Source slot.
    pub slot: usize,
}

/// Session actor registry behavior used by the bindings (donor
/// `SessionActorRegistry`).
pub trait QcActorRegistry {
    /// Whether the actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Source projection of a live actor.
    fn source_of(&self, actor: &ActorId) -> Option<ActorSource>;
    /// Owned handle of a live actor.
    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor>;
    /// Whether the handle is owned by this registry.
    fn is_owned(&self, actor: &OwnedActor) -> bool;
    /// Rebuild a saved reference within this registry.
    fn reference_saved(&self, saved: SavedActorId) -> ActorId;
}

/// Source actor slot behavior used by the bindings (donor
/// `SourceActorSlots`).
pub trait QcActorSlots {
    /// Owned actor at a slot.
    fn at(&self, slot: usize) -> Option<OwnedActor>;
    /// Slot storage provider (donor `slots.options.provider`).
    fn provider(&self) -> ProviderId;
    /// Whether the slot is free (donor
    /// `slots.options.storage.read(slot).free`).
    fn is_free(&self, slot: usize) -> bool;
}
