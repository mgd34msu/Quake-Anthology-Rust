//! QuakeC armor-pipeline and observer types (`src/content/q1/quakec/*`).
//!
//! Donor provenance: `src/contracts/gameplay.ts` (`ArmorDamageFlags`,
//! `ArmorStageInput`), `src/world/gameplay/armor.ts`
//! (`attackDamageFlags`), `src/world/gameplay/authority.ts`
//! (`SourceDamageResult`, `SourceDamageObserver`), and the used
//! `world/actors` registry/slot surface (`SessionActorRegistry`,
//! `SourceActorSlots`).
//!
//! The shared damage core (`DamageRequest`, `DamageOutcome`,
//! `AttackProvenance` with `AttackCause`, ...) lives in
//! [`crate::q1::foundation::gameplay`]; this module holds the
//! QuakeC-consumer pipeline and observer surface. Behavioral traits
//! that need [`QcError`](super::QcError) (`GameplayAuthority`,
//! `SourceArmorStage`) live in [`super::qc_view`].

use qa_core::identity::{ActorId, OwnedActor, ProviderId, SavedActorId};
use qa_core::math::Vec3;

use crate::contract::{decode_attack_damage_flags, ArmorState};

use super::super::foundation::gameplay::{AttackCause, DamageReaction, DamageRequest, Q1ArmorEffect};

/// Validated source damage result (donor `SourceDamageResult`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceDamageResult {
    /// Applied damage.
    pub applied_damage: f64,
    /// Reaction.
    pub reaction: DamageReaction,
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
        AttackCause::Q2 { damage_flags, .. } => *damage_flags,
        _ => 0,
    };
    let q3 = match cause {
        AttackCause::Q3 { damage_flags, .. } => *damage_flags,
        _ => 0,
    };
    let bypass = matches!(
        cause,
        AttackCause::Q1 { armor_effect, .. } if *armor_effect == Some(Q1ArmorEffect::Bypass)
    );
    let half = matches!(
        cause,
        AttackCause::Q1 { armor_effect, .. } if *armor_effect == Some(Q1ArmorEffect::HalfEffectiveness)
    );
    let bits = decode_attack_damage_flags(q2, q3, bypass, half);
    AttackDamageFlags {
        armor: ArmorDamageFlags {
            stage: None,
            no_armor: bits.no_armor,
            no_power_armor: bits.no_power_armor,
            no_regular_armor: bits.no_regular_armor,
            energy: bits.energy,
            regular_protection_scale: Some(if bits.half_protection { 0.5 } else { 1.0 }),
        },
        no_knockback: bits.no_knockback,
        no_protection: bits.no_protection,
        no_team_protection: bits.no_team_protection,
        destroy_armor: bits.destroy_armor,
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

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;
    use qa_core::time::SourceTime;

    use super::super::super::foundation::gameplay::{AttackProvenance, DamageDelivery};
    use super::super::super::foundation::types::ZERO;
    use super::*;

    fn request(cause: AttackCause) -> DamageRequest {
        let provider = ProviderId::new("q1", "test");
        let owner = IdentityOwner::create("test").expect("owner");
        DamageRequest {
            attack: AttackProvenance {
                sequence: 0,
                time: SourceTime::Seconds(0.0),
                attacker: None,
                inflictor: None,
                originating_projectile: None,
                weapon: None,
                weapon_provider: provider.clone(),
                damage_powerup_owner: None,
                combat_provider: provider.clone(),
                inventory_provider: provider.clone(),
                movement_provider: provider,
                cause,
            },
            target: owner.actor(1, 0),
            amount: 10.0,
            knockback: 0.0,
            direction: ZERO,
            point: ZERO,
            normal: ZERO,
            delivery: DamageDelivery::Direct,
        }
    }

    #[test]
    fn armor_flags_follow_q1_effect_and_q2_bits() {
        let bypass = attack_damage_flags(&request(AttackCause::Q1 {
            death_type: String::from("test"),
            armor_effect: Some(Q1ArmorEffect::Bypass),
        }));
        assert!(bypass.armor.no_armor);
        assert_eq!(bypass.armor.regular_protection_scale, Some(1.0));
        let half = attack_damage_flags(&request(AttackCause::Q1 {
            death_type: String::from("test"),
            armor_effect: Some(Q1ArmorEffect::HalfEffectiveness),
        }));
        assert!(!half.armor.no_armor);
        assert_eq!(half.armor.regular_protection_scale, Some(0.5));
        let q2 = attack_damage_flags(&request(AttackCause::Q2 {
            means_of_death: 1,
            damage_flags: 2 | 4 | 8,
            native: None,
        }));
        assert!(q2.armor.no_armor);
        assert!(q2.armor.energy);
        assert!(q2.no_knockback);
        assert!(!q2.no_protection);
        let q3 = attack_damage_flags(&request(AttackCause::Q3 {
            means_of_death: 1,
            damage_flags: 4 | 8 | 0x10,
        }));
        assert!(!q3.armor.no_armor);
        assert!(q3.no_knockback);
        assert!(q3.no_protection);
        assert!(q3.no_team_protection);
        assert!(!q3.destroy_armor);
    }
}
