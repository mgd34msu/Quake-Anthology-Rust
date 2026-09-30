//! Shared structural gameplay types for Q1 content.
//!
//! Donors: `src/contracts/gameplay.ts`, `src/contracts/world.ts` (actor,
//! body, and callback shapes), `src/world/gameplay/policies.ts`
//! (`Q1CombatContext`, `Q1DamageSourceEffects`),
//! `src/world/gameplay/damage-modifier.ts`
//! (`applySourceDamageModifier`), and `src/movement/q1/water-transition.ts`
//! (`q1WaterTransition`). These mirror the used surface structurally;
//! full ports belong to the world and movement lanes.

use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::math::{Bounds, Plane, Vec3};
use qa_core::time::SourceTime;

use crate::contract::{ArmorState, ItemId};
use crate::monsters::MonsterMission;

/// Attack cause (`AttackProvenance["cause"]`, donor `gameplay.ts`).
#[derive(Debug, Clone, PartialEq)]
pub enum AttackCause {
    /// Quake cause with a death type.
    Q1 {
        /// Death type text.
        death_type: String,
        /// Armor interaction override.
        armor_effect: Option<Q1ArmorEffect>,
    },
    /// Quake II cause with a canonical means of death.
    Q2 {
        /// Canonical means of death id.
        means_of_death: i32,
        /// Damage flags.
        damage_flags: i32,
        /// Original native cause, when captured.
        native: Option<Q2NativeCause>,
    },
    /// Quake III cause with a canonical means of death.
    Q3 {
        /// Canonical means of death id.
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

/// Q1 armor interaction override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1ArmorEffect {
    /// Bypass armor.
    Bypass,
    /// Halve armor effectiveness.
    HalfEffectiveness,
}

/// Original Q2 native cause (`Q2NativeCause`, donor `gameplay.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Q2NativeCause {
    /// Classic game DLL cause.
    Classic {
        /// Game tag.
        game: String,
        /// Native value.
        value: i32,
    },
    /// Rerelease cause.
    Rerelease {
        /// Cause id.
        id: i32,
        /// Friendly fire flag.
        friendly_fire: bool,
        /// No point loss flag.
        no_point_loss: bool,
    },
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
    /// Trigger.
    Trigger,
}

/// Damage delivery (`DamageRequest["delivery"]`, donor `gameplay.ts`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum DamageDelivery {
    /// Direct damage.
    #[default]
    Direct,
    /// Radius damage.
    Radius,
}

/// Captured attack provenance (`AttackProvenance`, donor `gameplay.ts`).
#[derive(Debug, Clone, PartialEq)]
pub struct AttackProvenance {
    /// Attack sequence number.
    pub sequence: u64,
    /// Attack time.
    pub time: SourceTime,
    /// Attacker, if any.
    pub attacker: Option<ActorId>,
    /// Inflictor, if any.
    pub inflictor: Option<ActorId>,
    /// Originating projectile, if any.
    pub originating_projectile: Option<ActorId>,
    /// Weapon item, if any.
    pub weapon: Option<ItemId>,
    /// Weapon provider.
    pub weapon_provider: ProviderId,
    /// Source that already applied its damage modifier.
    pub damage_powerup_owner: Option<ProviderId>,
    /// Combat provider.
    pub combat_provider: ProviderId,
    /// Inventory provider.
    pub inventory_provider: ProviderId,
    /// Movement provider.
    pub movement_provider: ProviderId,
    /// Attack cause.
    pub cause: AttackCause,
}

/// Damage request (`DamageRequest`, donor `gameplay.ts`).
#[derive(Debug, Clone, PartialEq)]
pub struct DamageRequest {
    /// Attack provenance.
    pub attack: AttackProvenance,
    /// Damage target.
    pub target: ActorId,
    /// Damage amount.
    pub amount: f64,
    /// Knockback amount.
    pub knockback: f64,
    /// Damage direction.
    pub direction: Vec3,
    /// Damage point.
    pub point: Vec3,
    /// Impact normal.
    pub normal: Vec3,
    /// Damage delivery.
    pub delivery: DamageDelivery,
}

/// Damage preparation verdict (`DamagePreparation`, donor `gameplay.ts`).
#[derive(Debug, Clone, PartialEq)]
pub enum DamagePreparation {
    /// Continue with an amount.
    Continue {
        /// Prepared amount.
        amount: f64,
    },
    /// Cancel the damage.
    Cancel,
}

/// Damage reaction (`DamageDecision["reaction"]`, donor `gameplay.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DamageReaction {
    /// No reaction.
    None,
    /// Pain reaction.
    Pain,
    /// Death reaction.
    Death,
}

/// Committed damage mutation (`DamageMutation`, donor `gameplay.ts`).
#[derive(Debug, Clone, PartialEq)]
pub enum DamageMutation {
    /// Health write.
    Health {
        /// Health before.
        before: f64,
        /// Health after.
        after: f64,
    },
    /// Armor write.
    Armor {
        /// Armor before.
        before: ArmorState,
        /// Armor after.
        after: ArmorState,
    },
    /// Source velocity write.
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

/// Per-game damage feedback (`DamageDecision["feedback"]`, donor
/// `gameplay.ts`).
#[derive(Debug, Clone, PartialEq)]
pub enum DamageFeedback {
    /// Quake II feedback.
    Q2 {
        /// Power armor saved.
        power_armor: f64,
        /// Armor saved.
        armor: f64,
        /// Blood.
        blood: f64,
        /// Knockback.
        knockback: f64,
    },
    /// Quake III feedback.
    Q3 {
        /// Knockback.
        knockback: f64,
        /// Battlesuit absorption.
        battlesuit: bool,
    },
}

/// Committed damage decision (`DamageDecision`, donor `gameplay.ts`).
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
    /// Source feedback, if any.
    pub feedback: Option<DamageFeedback>,
}

/// Damage outcome (`DamageOutcome`, donor `gameplay.ts`).
#[derive(Debug, Clone, PartialEq)]
pub enum DamageOutcome {
    /// The target retired before the decision.
    StaleTarget {
        /// Damage request.
        request: DamageRequest,
    },
    /// Committed decision.
    Committed {
        /// Damage decision.
        decision: DamageDecision,
        /// Whether the target survived.
        survived: bool,
    },
}

/// Live combat state (`CombatState`, donor `gameplay.ts`).
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
    /// Damage momentum immunity.
    pub no_knockback: Option<bool>,
    /// Team, if any.
    pub team: Option<String>,
}

/// Mutable combat traits (`CombatTraits`, donor
/// `world/gameplay/authority.ts`).
#[derive(Debug, Clone, PartialEq)]
pub struct CombatTraits {
    /// Whether the actor can take damage.
    pub can_take_damage: bool,
    /// Mass.
    pub mass: f64,
    /// Whether the actor is invulnerable.
    pub invulnerable: bool,
    /// Team, if any.
    pub team: Option<String>,
    /// Damage momentum immunity.
    pub no_knockback: Option<bool>,
}

/// Original attacker damage policy (`SourceDamageModifier`, donor
/// `gameplay.ts`).
pub struct SourceDamageModifier {
    /// Owning provider.
    pub owner: ProviderId,
    /// Amount transform over the actual attacker.
    pub transform: Q1DamageTransform,
}

impl std::fmt::Debug for SourceDamageModifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SourceDamageModifier")
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

/// Apply a source damage modifier, expiring dead attacker/inflictor
/// lifetimes to the original world context (`applySourceDamageModifier`,
/// donor `world/gameplay/damage-modifier.ts`).
pub fn apply_source_damage_modifier(
    request: DamageRequest,
    modifier: Option<&SourceDamageModifier>,
    is_live: &dyn Fn(&ActorId) -> bool,
) -> DamageRequest {
    let Some(modifier) = modifier else { return request };
    let current = |actor: Option<ActorId>| -> Option<ActorId> { actor.filter(|actor| is_live(actor)) };
    let mut attack = request.attack.clone();
    attack.attacker = current(attack.attacker.clone());
    attack.inflictor = current(attack.inflictor.clone());
    let amount = if attack.damage_powerup_owner.as_ref() == Some(&modifier.owner) {
        request.amount
    } else {
        (modifier.transform)(attack.attacker.as_ref(), request.amount)
    };
    attack.damage_powerup_owner = Some(modifier.owner.clone());
    DamageRequest {
        attack,
        amount,
        ..request
    }
}

/// Mission gate (`MissionGate`, donor `gameplay.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MissionGate {
    /// Objective id.
    pub objective: String,
    /// Whether the gate is satisfied.
    pub satisfied: bool,
}

/// Session travel intent (`TransitionIntent`, donor `gameplay.ts`).
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
        /// Travel cause, if any.
        cause: Option<ActorId>,
    },
    /// Complete a campaign.
    CampaignComplete {
        /// Campaign provider.
        campaign: ProviderId,
        /// Mission gates.
        gates: Vec<MissionGate>,
    },
    /// Complete a round.
    RoundComplete {
        /// Match provider.
        match_provider: ProviderId,
        /// Winner, if any.
        winner: Option<String>,
    },
    /// Rotate the match map.
    MatchRotation {
        /// Match provider.
        match_provider: ProviderId,
        /// Destination map.
        map: String,
    },
}

/// Authoritative body state (`BodyState`, donor `world.ts`).
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
    /// Ground actor, if any.
    pub ground: Option<ActorId>,
}

/// Body state patch with donor `Partial<BodyState>` semantics.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BodyPatch {
    /// Origin override.
    pub origin: Option<Vec3>,
    /// Angles override.
    pub angles: Option<Vec3>,
    /// Velocity override.
    pub velocity: Option<Vec3>,
    /// Bounds override.
    pub bounds: Option<Bounds>,
    /// Ground override (`None` clears the ground only when `clear_ground`
    /// is set... see `apply_to`).
    pub ground: Option<Option<ActorId>>,
}

impl BodyPatch {
    /// Apply the patch over a body state. A `ground` entry of
    /// `Some(None)` clears the ground; an absent entry keeps it.
    #[must_use]
    pub fn apply_to(&self, body: &BodyState) -> BodyState {
        BodyState {
            origin: self.origin.unwrap_or(body.origin),
            angles: self.angles.unwrap_or(body.angles),
            velocity: self.velocity.unwrap_or(body.velocity),
            bounds: self.bounds.unwrap_or(body.bounds),
            ground: self.ground.clone().unwrap_or_else(|| body.ground.clone()),
        }
    }
}

/// Touch surface (`TouchContact["surface"]`, donor `world.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TouchSurface {
    /// Surface name.
    pub name: String,
    /// Native surface flags.
    pub native_flags: i32,
    /// Native surface value.
    pub native_value: i32,
}

/// Synchronous touch contact (`TouchContact`, donor `world.ts`). The
/// Q2-rerelease source-trace variant is out of scope for Q1 content.
#[derive(Debug, Clone, PartialEq)]
pub struct TouchContact {
    /// Touching actor.
    pub self_actor: OwnedActor,
    /// Touched actor.
    pub other: ActorId,
    /// Contact plane, if any.
    pub plane: Option<Plane>,
    /// Contact surface, if any.
    pub surface: Option<TouchSurface>,
}

/// Linked body snapshot (`LinkedBody`, donor `world.ts`).
#[derive(Debug, Clone, PartialEq)]
pub struct LinkedBody {
    /// Linked actor.
    pub actor: ActorId,
    /// Linked state.
    pub state: BodyState,
    /// Absolute bounds captured by the last link.
    pub absolute_bounds: Bounds,
    /// Link counter.
    pub link_count: u64,
}

/// Body attachment (`BodyAttachment`, donor `world.ts`).
#[derive(Debug, Clone, PartialEq)]
pub struct BodyAttachment {
    /// Anchor actor.
    pub anchor: ActorId,
    /// Follow mode.
    pub follow: BodyFollow,
}

/// Body follow mode.
#[derive(Debug, Clone, PartialEq)]
pub enum BodyFollow {
    /// Follow translations with an offset.
    Translation {
        /// Follow offset.
        offset: Vec3,
    },
    /// Follow the anchor center.
    Center,
    /// Follow the bounds minimum with an offset.
    BoundsMin {
        /// Follow offset.
        offset: Vec3,
    },
}

/// Actor observation (`ActorObservation`, donor `world.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActorObservation {
    /// Observed actor.
    pub id: ActorId,
    /// Owning provider.
    pub owner: ProviderId,
    /// Spawn definition.
    pub definition: String,
}

/// Source slot binding (`sourceOf`, donor `world/actors/registry.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceSlot {
    /// Owning provider.
    pub provider: ProviderId,
    /// Source slot.
    pub slot: u32,
}

/// Pain reaction (`PainReaction`, donor `world.ts`).
#[derive(Debug, Clone, PartialEq)]
pub struct PainReaction {
    /// Attack provenance, if any.
    pub attack: Option<AttackProvenance>,
    /// Reacting actor.
    pub self_actor: OwnedActor,
    /// Attacker, if any.
    pub attacker: Option<ActorId>,
    /// Armor kick.
    pub kick: f64,
    /// Damage dealt.
    pub damage: f64,
}

/// Death reaction (`DeathReaction`, donor `world.ts`).
#[derive(Debug, Clone, PartialEq)]
pub struct DeathReaction {
    /// Attack provenance, if any.
    pub attack: Option<AttackProvenance>,
    /// Reacting actor.
    pub self_actor: OwnedActor,
    /// Attacker, if any.
    pub attacker: Option<ActorId>,
    /// Armor kick.
    pub kick: f64,
    /// Damage dealt.
    pub damage: f64,
    /// Inflictor, if any.
    pub inflictor: Option<ActorId>,
    /// Death point.
    pub point: Vec3,
}

/// Think frame times used by Q1 bindings (`FrameContext` time/elapsed,
/// donor `time.ts`). Frame counters and phases stay engine-side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1ThinkFrame {
    /// Frame time.
    pub time: SourceTime,
    /// Elapsed frame time.
    pub elapsed: SourceTime,
}

/// Combat arithmetic profile (`Arithmetic`, donor
/// `world/gameplay/policies.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1CombatArithmetic {
    /// Binary32 each-operation rounding.
    Binary32,
    /// Binary64 donor rounding.
    Binary64,
}

/// Q1 combat policy context (`Q1CombatContext`, donor
/// `world/gameplay/policies.ts`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1CombatContext {
    /// Combat arithmetic.
    pub arithmetic: Q1CombatArithmetic,
    /// Whether quad damage applies.
    pub quad: bool,
    /// Teamplay mode.
    pub teamplay: i32,
    /// Whether base team-health rules apply.
    pub base_team_health: bool,
    /// Whether the target walks.
    pub walk: bool,
    /// Momentum direction, if any.
    pub momentum_direction: Option<Vec3>,
}

/// Lethal-health stage result (`lethalHealth` return, donor
/// `world/gameplay/policies.ts`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1LethalHealth {
    /// Proposed health.
    pub health: f64,
    /// Reaction.
    pub reaction: Q1LethalReaction,
}

/// Lethal-health reaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1LethalReaction {
    /// Suppress death.
    None,
    /// Die.
    Death,
}

/// Damage amount transform over the actual attacker.
pub type Q1DamageTransform = Box<dyn Fn(Option<&ActorId>, f64) -> f64>;
/// Damage stage over request, amount, and combat states.
pub type Q1DamageStage<T> = Box<dyn Fn(&DamageRequest, f64, &CombatState, Option<&CombatState>) -> T>;
/// Damage stage over request and combat states.
pub type Q1DamageStateStage<T> = Box<dyn Fn(&DamageRequest, &CombatState, Option<&CombatState>) -> T>;

/// Ordered source damage stages (`Q1DamageSourceEffects`, donor
/// `world/gameplay/policies.ts`).
#[derive(Default)]
pub struct Q1DamageSourceEffects {
    /// Quad preparation stage.
    pub before_quad: Option<Q1DamageStage<DamagePreparation>>,
    /// Post-quad preparation stage.
    pub after_quad: Option<Q1DamageStage<DamagePreparation>>,
    /// Armor permission stage.
    pub armor_allowed: Option<Q1DamageStage<bool>>,
    /// Protection permission stage.
    pub protection_applies: Option<Q1DamageStateStage<bool>>,
    /// Pre-health permission stage.
    pub before_health: Option<Q1DamageStage<bool>>,
    /// Post-armor scaling stage.
    pub after_armor: Option<Q1DamageStage<f64>>,
    /// Lethal-health stage.
    pub lethal_health: Option<Q1DamageStage<Q1LethalHealth>>,
}

impl std::fmt::Debug for Q1DamageSourceEffects {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Q1DamageSourceEffects").finish_non_exhaustive()
    }
}

/// Water transition result (`q1WaterTransition` return, donor
/// `movement/q1/water-transition.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Q1WaterTransition {
    /// New water type.
    pub water_type: i32,
    /// New water level.
    pub water_level: i32,
    /// Whether the transition splashes.
    pub splash: bool,
}

/// SV_CheckWaterTransition with source initialization values
/// (`q1WaterTransition`, donor `movement/q1/water-transition.ts`).
#[must_use]
pub fn q1_water_transition(previous_type: i32, contents: i32) -> Q1WaterTransition {
    if previous_type == 0 {
        return Q1WaterTransition {
            water_type: contents,
            water_level: 1,
            splash: false,
        };
    }
    if contents <= -3 {
        return Q1WaterTransition {
            water_type: contents,
            water_level: 1,
            splash: previous_type == -1,
        };
    }
    Q1WaterTransition {
        water_type: -1,
        water_level: contents,
        splash: previous_type != -1,
    }
}

/// Authored monster mission callbacks with the donor ambush flag
/// (`MonsterMission`, donor `src/content/monsters/authored.ts`). The
/// shared [`MonsterMission`] trait carries the callbacks; this
/// extension adds the ambush flag Q1 reads without touching the
/// shared module.
pub trait Q1MonsterMission: MonsterMission {
    /// Whether the monster ambushes instead of sharing sightings.
    fn ambush(&self) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn water_transition_matches_source_initialization() {
        assert_eq!(
            q1_water_transition(0, -3),
            Q1WaterTransition {
                water_type: -3,
                water_level: 1,
                splash: false
            }
        );
        assert_eq!(
            q1_water_transition(-1, -5),
            Q1WaterTransition {
                water_type: -5,
                water_level: 1,
                splash: true
            }
        );
        assert_eq!(
            q1_water_transition(-3, -1),
            Q1WaterTransition {
                water_type: -1,
                water_level: -1,
                splash: true
            }
        );
    }

    #[test]
    fn damage_modifier_expires_dead_attackers() {
        let owner = ProviderId::new("q1", "test");
        let request = DamageRequest {
            attack: AttackProvenance {
                sequence: 1,
                time: SourceTime::Seconds(1.0),
                attacker: None,
                inflictor: None,
                originating_projectile: None,
                weapon: None,
                weapon_provider: owner.clone(),
                damage_powerup_owner: None,
                combat_provider: owner.clone(),
                inventory_provider: owner.clone(),
                movement_provider: owner.clone(),
                cause: AttackCause::Q1 {
                    death_type: String::new(),
                    armor_effect: None,
                },
            },
            target: qa_core::identity::IdentityOwner::create("test")
                .expect("owner")
                .actor(1, 0),
            amount: 10.0,
            knockback: 10.0,
            direction: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            point: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            delivery: DamageDelivery::Direct,
        };
        let unchanged = apply_source_damage_modifier(request.clone(), None, &|_| true);
        assert_eq!(unchanged.amount, 10.0);
        assert_eq!(unchanged.attack.damage_powerup_owner, None);
    }
}
