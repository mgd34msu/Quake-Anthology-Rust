//! Combat rules ported from `src/world/gameplay/armor.ts`,
//! `damage-modifier.ts`, and the `policies.ts` ordering notes. Armor
//! absorption, flag decoding, health floors, knockback scaling, and Q1 aim
//! live here; source reentrancy (staged continuations, pain/death hooks)
//! stays with the owning game provider.

use qa_core::identity::{ActorId, ProviderId};
use qa_core::math::{vec3, Bounds, Vec3};

use crate::WorldError;

/// Item identifier (`namespace:name`).
pub type ItemId = String;

/// Build an item identifier.
#[must_use]
pub fn item_id(namespace: &str, name: &str) -> ItemId {
    format!("{namespace}:{name}")
}

/// Regular armor state.
#[derive(Debug, Clone, PartialEq)]
pub enum RegularArmor {
    /// No armor.
    None,
    /// Quake I armor with absorption fraction.
    Q1 {
        /// Armor points.
        points: f64,
        /// Absorption fraction.
        absorption: f64,
        /// Source item.
        item: ItemId,
    },
    /// Quake II armor with normal/energy protection.
    Q2 {
        /// Armor points.
        points: f64,
        /// Normal protection fraction.
        normal_protection: f64,
        /// Energy protection fraction.
        energy_protection: f64,
        /// Source item.
        item: ItemId,
    },
    /// Quake III armor with a single protection fraction.
    Q3 {
        /// Armor points.
        points: f64,
        /// Protection fraction.
        protection: f64,
    },
    /// Source-owned armor with an external absorption binding.
    Source {
        /// Armor points.
        points: f64,
        /// Source item, if any.
        item: Option<ItemId>,
    },
}

/// Powered protection state.
#[derive(Debug, Clone, PartialEq)]
pub enum PoweredProtection {
    /// No powered protection.
    None,
    /// Screen with cell count.
    Screen {
        /// Remaining cells.
        cells: i32,
    },
    /// Shield with cell count.
    Shield {
        /// Remaining cells.
        cells: i32,
    },
}

/// Combined armor state.
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorState {
    /// Regular armor.
    pub regular: RegularArmor,
    /// Powered protection.
    pub powered: PoweredProtection,
}

/// Armor evaluation stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmorStage {
    /// Power stage.
    Power,
    /// Regular stage.
    Regular,
}

/// Decoded damage flags for one armor evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArmorDamageFlags {
    /// Evaluation stage, if staged.
    pub stage: Option<ArmorStage>,
    /// Skip all armor.
    pub no_armor: bool,
    /// Skip power armor.
    pub no_power_armor: bool,
    /// Skip regular armor.
    pub no_regular_armor: bool,
    /// Energy damage (Q2).
    pub energy: bool,
    /// Regular protection scale.
    pub regular_protection_scale: f64,
}

/// Q2 victim profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2ArmorProduct {
    /// Classic.
    Classic,
    /// Rerelease.
    Rerelease,
}

/// Q2 victim context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2ArmorContext {
    /// Source product.
    pub product: Q2ArmorProduct,
    /// Capture-the-flag rules.
    pub ctf: bool,
    /// Victim alive.
    pub alive: bool,
}

/// Armor arithmetic selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmorArithmetic {
    /// Round every multiply to binary32.
    Binary32,
    /// Binary64 intermediates.
    Binary64,
}

/// Victim context for native armor absorption.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VictimArmorContext {
    /// Screen facing dot (`normalize(point - origin) . forward`).
    pub screen_facing_dot: f64,
    /// Multiply arithmetic.
    pub arithmetic: ArmorArithmetic,
    /// Q2 profile, required for Q2 armor stages.
    pub q2: Option<Q2ArmorContext>,
}

/// Armor absorption result.
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorResult {
    /// Updated armor.
    pub armor: ArmorState,
    /// Damage saved by power armor.
    pub power_saved: f64,
    /// Damage saved by regular armor.
    pub regular_saved: f64,
}

/// Native armor absorption across Q1/Q2/Q3 formulas.
pub fn absorb_native_armor(
    armor: &ArmorState,
    damage: f64,
    flags: &ArmorDamageFlags,
    context: &VictimArmorContext,
) -> Result<ArmorResult, WorldError> {
    let needs_q2 = (flags.stage != Some(ArmorStage::Power) && matches!(armor.regular, RegularArmor::Q2 { .. })
        || flags.stage != Some(ArmorStage::Regular) && !matches!(armor.powered, PoweredProtection::None))
        && context.q2.is_none();
    if needs_q2 {
        return Err(WorldError::ArmorProfile);
    }
    if damage == 0.0
        || flags.no_armor
        || (matches!(armor.regular, RegularArmor::None) && matches!(armor.powered, PoweredProtection::None))
    {
        return Ok(ArmorResult {
            armor: armor.clone(),
            power_saved: 0.0,
            regular_saved: 0.0,
        });
    }
    let multiply = |left: f64, right: f64| {
        if context.arithmetic == ArmorArithmetic::Binary32 {
            f64::from((left as f32) * (right as f32))
        } else {
            left * right
        }
    };
    let protection_scale = flags.regular_protection_scale;
    let mut power_saved = 0.0;
    let mut powered = armor.powered.clone();
    let q2 = context.q2;
    let rerelease = q2.is_some_and(|profile| profile.product == Q2ArmorProduct::Rerelease);
    let facing_limit = if rerelease { f64::from(0.3_f32) } else { 0.3 };
    let power_armed = flags.stage != Some(ArmorStage::Regular)
        && !flags.no_power_armor
        && (!rerelease || q2.is_some_and(|profile| profile.alive))
        && !matches!(powered, PoweredProtection::None)
        && powered_cells(&powered) > 0
        && (!matches!(powered, PoweredProtection::Screen { .. }) || context.screen_facing_dot > facing_limit);
    if power_armed {
        let screen = matches!(powered, PoweredProtection::Screen { .. });
        let cells = powered_cells(&powered);
        let damage_per_cell = if screen || q2.is_some_and(|profile| profile.ctf) {
            1
        } else {
            2
        };
        let divided = if screen {
            (damage / 3.0).trunc()
        } else {
            ((2.0 * damage) / 3.0).trunc()
        };
        let protected = if rerelease { divided.max(1.0) } else { divided };
        let doubled_cost = if rerelease {
            flags.energy
        } else {
            flags.no_regular_armor
        };
        let base_available = f64::from(cells * damage_per_cell);
        let divided_available = if doubled_cost {
            (base_available / 2.0).trunc()
        } else {
            base_available
        };
        let available = if rerelease {
            divided_available.max(1.0)
        } else {
            divided_available
        };
        power_saved = available.min(protected);
        let used = (power_saved / f64::from(damage_per_cell)).trunc() as i32 * if doubled_cost { 2 } else { 1 };
        let remaining = if rerelease {
            (cells - damage_per_cell.max(used)).max(0)
        } else {
            cells - used
        };
        powered = match powered {
            PoweredProtection::Screen { .. } => PoweredProtection::Screen { cells: remaining },
            PoweredProtection::Shield { .. } => PoweredProtection::Shield { cells: remaining },
            PoweredProtection::None => PoweredProtection::None,
        };
    }
    let mut regular = armor.regular.clone();
    let mut regular_saved = 0.0;
    if !flags.no_regular_armor && flags.stage != Some(ArmorStage::Power) {
        match &regular {
            RegularArmor::None => {}
            RegularArmor::Source { .. } => return Err(WorldError::SourceArmor),
            RegularArmor::Q1 {
                points,
                absorption,
                item,
            } => {
                let saved = points.min(multiply(multiply(*absorption, protection_scale), damage - power_saved).ceil());
                let depleted = saved >= *points;
                regular = RegularArmor::Q1 {
                    points: points - saved,
                    absorption: if depleted { 0.0 } else { *absorption },
                    item: item.clone(),
                };
                regular_saved = saved;
            }
            RegularArmor::Q2 {
                points,
                normal_protection,
                energy_protection,
                item,
            } => {
                let protection = if flags.energy {
                    *energy_protection
                } else {
                    *normal_protection
                };
                let saved = points.min(multiply(multiply(protection, protection_scale), damage - power_saved).ceil());
                regular = RegularArmor::Q2 {
                    points: points - saved,
                    normal_protection: *normal_protection,
                    energy_protection: *energy_protection,
                    item: item.clone(),
                };
                regular_saved = saved;
            }
            RegularArmor::Q3 { points, protection } => {
                let scaled =
                    f64::from((damage - power_saved) as f32 * ((*protection as f32) * (protection_scale as f32)));
                let saved = points.min(scaled.ceil());
                regular = RegularArmor::Q3 {
                    points: points - saved,
                    protection: *protection,
                };
                regular_saved = saved;
            }
        }
    }
    Ok(ArmorResult {
        armor: ArmorState { regular, powered },
        power_saved,
        regular_saved,
    })
}

fn powered_cells(powered: &PoweredProtection) -> i32 {
    match powered {
        PoweredProtection::None => 0,
        PoweredProtection::Screen { cells } | PoweredProtection::Shield { cells } => *cells,
    }
}

/// Q1 armor effect from the damage cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1ArmorEffect {
    /// Full armor.
    Full,
    /// Bypass armor.
    Bypass,
    /// Half effectiveness.
    Half,
}

/// Damage cause in its native namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DamageCause {
    /// Quake I cause.
    Q1 {
        /// Armor effect.
        armor_effect: Q1ArmorEffect,
    },
    /// Quake II cause with native damage flags.
    Q2 {
        /// Native damage flags.
        damage_flags: i32,
    },
    /// Quake III cause with native damage flags.
    Q3 {
        /// Native damage flags.
        damage_flags: i32,
    },
}

/// Decoded attack flags. Native flags decode by origin, never as another
/// game's bit positions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttackFlags {
    /// Armor flags for absorption.
    pub armor: ArmorDamageFlags,
    /// No knockback.
    pub no_knockback: bool,
    /// No protection.
    pub no_protection: bool,
    /// No team protection.
    pub no_team_protection: bool,
    /// Destroy armor (take equals full damage).
    pub destroy_armor: bool,
}

/// Decode native damage flags for one attack.
#[must_use]
pub fn attack_damage_flags(cause: &DamageCause) -> AttackFlags {
    let q2 = match cause {
        DamageCause::Q2 { damage_flags } => *damage_flags,
        _ => 0,
    };
    let q3 = match cause {
        DamageCause::Q3 { damage_flags } => *damage_flags,
        _ => 0,
    };
    let bypass = matches!(
        cause,
        DamageCause::Q1 {
            armor_effect: Q1ArmorEffect::Bypass
        }
    );
    let half = matches!(
        cause,
        DamageCause::Q1 {
            armor_effect: Q1ArmorEffect::Half
        }
    );
    AttackFlags {
        armor: ArmorDamageFlags {
            stage: None,
            no_armor: (q2 | q3) & 2 != 0 || bypass,
            no_power_armor: q2 & 0x100 != 0,
            no_regular_armor: q2 & 0x80 != 0,
            energy: q2 & 4 != 0,
            regular_protection_scale: if half { 0.5 } else { 1.0 },
        },
        no_knockback: q2 & 8 != 0 || q3 & 4 != 0,
        no_protection: q2 & 0x20 != 0 || q3 & 8 != 0,
        no_team_protection: q3 & 0x10 != 0,
        destroy_armor: q2 & 0x40 != 0,
    }
}

/// Damage delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Direct hit.
    Direct,
    /// Radius damage.
    Radius,
}

/// Damage reaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reaction {
    /// No reaction.
    None,
    /// Pain.
    Pain,
    /// Death.
    Death,
}

/// Combat state of one actor.
#[derive(Debug, Clone, PartialEq)]
pub struct CombatState {
    /// Health.
    pub health: f64,
    /// Armor.
    pub armor: ArmorState,
    /// Mass for knockback scaling.
    pub mass: f64,
    /// Whether damage applies.
    pub can_take_damage: bool,
    /// Invulnerability (protection).
    pub invulnerable: bool,
    /// Immunity to damage momentum.
    pub no_knockback: bool,
    /// Team name, if any.
    pub team: Option<String>,
}

impl Default for CombatState {
    fn default() -> Self {
        Self {
            health: 100.0,
            armor: ArmorState {
                regular: RegularArmor::None,
                powered: PoweredProtection::None,
            },
            mass: 200.0,
            can_take_damage: true,
            invulnerable: false,
            no_knockback: false,
            team: None,
        }
    }
}

/// Apply a source damage modifier once. An already-applied owner keeps the
/// original amount, including an unchanged result.
#[must_use]
pub fn apply_damage_modifier(
    amount: f64,
    attacker: Option<&ActorId>,
    applied_owner: Option<&ProviderId>,
    modifier_owner: &ProviderId,
    transform: &dyn Fn(Option<&ActorId>, f64) -> f64,
) -> (f64, ProviderId) {
    if applied_owner == Some(modifier_owner) {
        (amount, modifier_owner.clone())
    } else {
        (transform(attacker, amount), modifier_owner.clone())
    }
}

/// Q1 health floor is `-99`.
#[must_use]
pub fn q1_health_take(health: f64, take: f64) -> f64 {
    (health - take).max(-99.0)
}

/// Q2 health floor is `-999` with truncation.
#[must_use]
pub fn q2_health_take(health: f64, take: f64) -> f64 {
    (health - take).trunc().max(-999.0)
}

/// Q3 health floor is `-999` with `| 0` conversion.
#[must_use]
pub fn q3_health_take(health: f64, take: f64) -> f64 {
    f64::from((health - take).trunc() as i32).max(-999.0)
}

/// Reaction for committed health.
#[must_use]
pub fn reaction_for_health(health: f64, suppress_pain: bool) -> Reaction {
    if health <= 0.0 {
        Reaction::Death
    } else if suppress_pain {
        Reaction::None
    } else {
        Reaction::Pain
    }
}

/// Directed knockback impulse with binary32 rounding.
#[must_use]
pub fn directed_impulse_f32(direction: Vec3, amount: f32) -> Vec3 {
    let length =
        (f64::from(direction.x * direction.x + direction.y * direction.y + direction.z * direction.z).sqrt()) as f32;
    if length == 0.0 {
        return vec3(0.0, 0.0, 0.0);
    }
    let inverse = 1.0 / length;
    vec3(
        direction.x * inverse * amount,
        direction.y * inverse * amount,
        direction.z * inverse * amount,
    )
}

/// Q2 knockback impulse: `500 * knockback / max(50, mass)`, `1600` for
/// player self-damage.
#[must_use]
pub fn q2_knockback_impulse(direction: Vec3, knockback: f64, mass: f64, player_self: bool) -> Vec3 {
    let coefficient = if player_self { 1600.0 } else { 500.0 };
    let amount = ((coefficient * knockback.trunc()) / mass.max(50.0)) as f32;
    directed_impulse_f32(direction, amount)
}

/// Q3 knockback impulse: `min(damage, 200)`, scaled by `knockback_scale`.
#[must_use]
pub fn q3_knockback_impulse(direction: Vec3, damage: f64, knockback_scale: f32) -> (Vec3, f32) {
    let knockback = damage.trunc().clamp(0.0, 200.0) as f32;
    let amount = knockback_scale * knockback / 200.0;
    (directed_impulse_f32(direction, amount), knockback)
}

/// Q1 aim targets for `PF_aim`-style selection.
pub struct Q1AimTargets<'a> {
    /// Candidate targets in native source slot order.
    pub targets: &'a [ActorId],
    /// Body lookup.
    pub body: &'a dyn Fn(&ActorId) -> Option<(Vec3, Bounds)>,
    /// Eligibility lookup.
    pub eligible: &'a dyn Fn(&ActorId) -> bool,
    /// Blocked-trace lookup, returning the first blocking actor.
    pub trace: &'a dyn Fn(Vec3, Vec3) -> Option<ActorId>,
}

/// Q1 aim selection. The speed argument is unused upstream; target height
/// correction preserves horizontal aim. Equal alignment selects the later
/// visible target.
#[must_use]
pub fn aim_q1(origin: Vec3, forward: Vec3, threshold: f64, targets: &Q1AimTargets) -> Vec3 {
    let start = vec3(origin.x, origin.y, origin.z + 20.0);
    let far = vec3(
        start.x + forward.x * 2048.0,
        start.y + forward.y * 2048.0,
        start.z + forward.z * 2048.0,
    );
    if let Some(hit) = (targets.trace)(start, far) {
        if (targets.eligible)(&hit) {
            return forward;
        }
    }
    let mut best = threshold;
    let mut selected: Option<Vec3> = None;
    for actor in targets.targets {
        if !(targets.eligible)(actor) {
            continue;
        }
        let Some((actor_origin, bounds)) = (targets.body)(actor) else {
            continue;
        };
        let aim = vec3(
            actor_origin.x + 0.5 * (bounds.min.x + bounds.max.x),
            actor_origin.y + 0.5 * (bounds.min.y + bounds.max.y),
            actor_origin.z + 0.5 * (bounds.min.z + bounds.max.z),
        );
        let delta = vec3(aim.x - start.x, aim.y - start.y, aim.z - start.z);
        let length = (f64::from(delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt()) as f32;
        if length == 0.0 {
            continue;
        }
        let direction = vec3(delta.x / length, delta.y / length, delta.z / length);
        let alignment = f64::from(direction.x * forward.x + direction.y * forward.y + direction.z * forward.z);
        if alignment < best {
            continue;
        }
        if let Some(hit) = (targets.trace)(start, aim) {
            if hit == *actor {
                best = alignment;
                selected = Some(actor_origin);
            }
        }
    }
    let Some(selected) = selected else {
        return forward;
    };
    let delta = vec3(selected.x - origin.x, selected.y - origin.y, selected.z - origin.z);
    let along = delta.x * forward.x + delta.y * forward.y + delta.z * forward.z;
    let mut corrected = vec3(forward.x * along, forward.y * along, delta.z);
    let length =
        (f64::from(corrected.x * corrected.x + corrected.y * corrected.y + corrected.z * corrected.z).sqrt()) as f32;
    if length != 0.0 {
        corrected = vec3(corrected.x / length, corrected.y / length, corrected.z / length);
    }
    corrected
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::{vec3, Bounds};

    fn flags() -> ArmorDamageFlags {
        ArmorDamageFlags {
            stage: None,
            no_armor: false,
            no_power_armor: false,
            no_regular_armor: false,
            energy: false,
            regular_protection_scale: 1.0,
        }
    }

    fn test_context() -> VictimArmorContext {
        VictimArmorContext {
            screen_facing_dot: 1.0,
            arithmetic: ArmorArithmetic::Binary64,
            q2: None,
        }
    }

    #[test]
    fn q1_armor_absorbs_and_depletes() {
        let armor = ArmorState {
            regular: RegularArmor::Q1 {
                points: 100.0,
                absorption: 0.3,
                item: item_id("q1", "armor"),
            },
            powered: PoweredProtection::None,
        };
        let result = absorb_native_armor(&armor, 100.0, &flags(), &test_context()).unwrap();
        assert_eq!(result.regular_saved, 30.0);
        assert_eq!(result.power_saved, 0.0);
        let depleted = absorb_native_armor(
            &ArmorState {
                regular: RegularArmor::Q1 {
                    points: 10.0,
                    absorption: 0.3,
                    item: item_id("q1", "armor"),
                },
                powered: PoweredProtection::None,
            },
            100.0,
            &flags(),
            &test_context(),
        )
        .unwrap();
        assert_eq!(depleted.regular_saved, 10.0);
        assert!(matches!(
            depleted.armor.regular,
            RegularArmor::Q1 { absorption: 0.0, .. }
        ));
    }

    #[test]
    fn q2_and_q3_armor_use_their_protection_paths() {
        let q2 = ArmorState {
            regular: RegularArmor::Q2 {
                points: 50.0,
                normal_protection: 0.6,
                energy_protection: 0.8,
                item: item_id("q2", "jacket"),
            },
            powered: PoweredProtection::None,
        };
        let context = VictimArmorContext {
            q2: Some(Q2ArmorContext {
                product: Q2ArmorProduct::Classic,
                ctf: false,
                alive: true,
            }),
            ..test_context()
        };
        let result = absorb_native_armor(&q2, 100.0, &flags(), &context).unwrap();
        assert_eq!(result.regular_saved, 50.0);
        let q3 = ArmorState {
            regular: RegularArmor::Q3 {
                points: 200.0,
                protection: 0.5,
            },
            powered: PoweredProtection::None,
        };
        let result = absorb_native_armor(&q3, 100.0, &flags(), &test_context()).unwrap();
        assert_eq!(result.regular_saved, 50.0);
    }

    #[test]
    fn flags_decode_by_origin() {
        let q2 = attack_damage_flags(&DamageCause::Q2 { damage_flags: 8 | 0x80 });
        assert!(q2.no_knockback);
        assert!(q2.armor.no_regular_armor);
        assert!(!q2.armor.no_armor);
        let bypass = attack_damage_flags(&DamageCause::Q1 {
            armor_effect: Q1ArmorEffect::Bypass,
        });
        assert!(bypass.armor.no_armor);
        let half = attack_damage_flags(&DamageCause::Q1 {
            armor_effect: Q1ArmorEffect::Half,
        });
        assert_eq!(half.armor.regular_protection_scale, 0.5);
    }

    #[test]
    fn health_floors_and_reactions_match_sources() {
        assert_eq!(q1_health_take(10.0, 200.0), -99.0);
        assert_eq!(q2_health_take(10.5, 3.0), 7.0);
        assert_eq!(q3_health_take(10.0, 2000.0), -999.0);
        assert_eq!(reaction_for_health(-1.0, false), Reaction::Death);
        assert_eq!(reaction_for_health(5.0, true), Reaction::None);
        assert_eq!(reaction_for_health(5.0, false), Reaction::Pain);
    }

    #[test]
    fn knockback_scales_by_family() {
        let q2 = q2_knockback_impulse(vec3(1.0, 0.0, 0.0), 100.0, 200.0, false);
        assert!((q2.x - 250.0).abs() < 0.01);
        let slf = q2_knockback_impulse(vec3(1.0, 0.0, 0.0), 100.0, 200.0, true);
        assert!((slf.x - 800.0).abs() < 0.01);
        let (q3, knockback) = q3_knockback_impulse(vec3(0.0, 1.0, 0.0), 300.0, 1000.0);
        assert_eq!(knockback, 200.0);
        assert!((q3.y - 1000.0).abs() < 0.01);
    }

    #[test]
    fn q1_aim_prefers_aligned_visible_targets() {
        let origin = vec3(0.0, 0.0, 0.0);
        let forward = vec3(1.0, 0.0, 0.0);
        let owner = qa_core::identity::IdentityOwner::create("aim").unwrap();
        let aligned = owner.actor(1, 0);
        let body = |actor: &ActorId| {
            if *actor == aligned {
                Some((
                    vec3(100.0, 0.0, 0.0),
                    Bounds {
                        min: vec3(-8.0, -8.0, -8.0),
                        max: vec3(8.0, 8.0, 8.0),
                    },
                ))
            } else {
                None
            }
        };
        let eligible = |_: &ActorId| true;
        let trace = |_: Vec3, _: Vec3| None;
        let targets = Q1AimTargets {
            targets: std::slice::from_ref(&aligned),
            body: &body,
            eligible: &eligible,
            trace: &trace,
        };
        let aimed = aim_q1(origin, forward, 0.9, &targets);
        assert!((aimed.x - 1.0).abs() < 0.2);
    }
}
