pub(crate) use qa_core::math::{difference, length, normalized};
use qa_core::primitives::{Body, CallbackId, DamageEvent, DamageFlags, EntityId, Vec3};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Motion {
    #[default]
    None,
    Walk,
    Fly,
    Toss,
    Bounce,
    Push,
    Stop,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PowerArmor {
    #[default]
    None,
    Screen,
    Shield,
}

#[derive(Clone, Copy, Debug)]
pub struct CombatTraits {
    pub damageable: bool,
    pub client: bool,
    pub monster: bool,
    pub god: bool,
    pub invincible: bool,
    pub noclip: bool,
    pub battlesuit: bool,
    pub no_knockback: bool,
    pub ducked: bool,
    pub has_enemy: bool,
    pub motion: Motion,
    pub mass: f32,
    pub position: Vec3,
    pub forward: Vec3,
    pub power_armor: PowerArmor,
    pub knockback_time_ms: i32,
    pub pain: Option<CallbackId>,
    pub die: Option<CallbackId>,
}

impl Default for CombatTraits {
    fn default() -> Self {
        Self {
            damageable: true,
            client: false,
            monster: false,
            god: false,
            invincible: false,
            noclip: false,
            battlesuit: false,
            no_knockback: false,
            ducked: false,
            has_enemy: false,
            motion: Motion::None,
            mass: 200.0,
            position: Vec3::default(),
            forward: Vec3([1.0, 0.0, 0.0]),
            power_armor: PowerArmor::None,
            knockback_time_ms: 0,
            pain: None,
            die: None,
        }
    }
}

/// Borrows the authoritative player fields or NPC columns. No combat shadow state.
pub struct DamageTarget<'a> {
    pub health: &'a mut i32,
    pub armor: &'a mut i32,
    pub absorption: &'a mut f32,
    pub energy_absorption: f32,
    pub power_cells: &'a mut i32,
    pub velocity: &'a mut Vec3,
    pub traits: &'a mut CombatTraits,
}

#[derive(Clone, Copy, Debug)]
pub struct DamageContext {
    pub self_hit: bool,
    pub same_team: bool,
    pub prevent_team_damage: bool,
    pub easy_single_player: bool,
    pub attacker_client: bool,
    pub attacker_handicap: i32,
    pub attacker_multiplier: f32,
    pub knockback_scale: f32,
    pub inflictor_center: Vec3,
    pub intermission: bool,
}

impl Default for DamageContext {
    fn default() -> Self {
        Self {
            self_hit: false,
            same_team: false,
            prevent_team_damage: false,
            easy_single_player: false,
            attacker_client: false,
            attacker_handicap: 100,
            attacker_multiplier: 1.0,
            knockback_scale: 1000.0,
            inflictor_center: Vec3::default(),
            intermission: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reaction {
    #[default]
    None,
    Pain,
    Death,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DamageResult {
    pub health_damage: i32,
    pub armor_saved: i32,
    pub power_saved: i32,
    pub knockback: i32,
    pub reaction: Reaction,
    pub blocked: bool,
    pub disable_damage: bool,
    pub disable_knockback: bool,
}

/// Chosen when the character is loaded, independently of map and weapon rules.
pub struct DamageRules {
    pub(crate) prepare: fn(&mut DamageTarget<'_>, DamageEvent, DamageContext) -> DamageResult,
    pub(crate) minimum_health: i32,
    pub(crate) radius: fn(Blast, BlastTarget) -> Option<(f32, Vec3)>,
}

pub fn apply_damage(
    rules: &DamageRules,
    target: &mut DamageTarget<'_>,
    mut event: DamageEvent,
    context: DamageContext,
    mut dispatch: impl FnMut(CallbackId, &mut DamageTarget<'_>, DamageEvent, DamageResult),
) -> DamageResult {
    if !target.traits.damageable {
        return DamageResult::default();
    }
    // Attacker powerups act once, before the target's absorption rules.
    event.amount *= context.attacker_multiplier;
    let result = (rules.prepare)(target, event, context);
    *target.health -= result.health_damage;
    if result.reaction == Reaction::Death {
        *target.health = (*target.health).max(rules.minimum_health);
        if result.disable_damage {
            target.traits.damageable = false;
        }
        if result.disable_knockback {
            target.traits.no_knockback = true;
        }
    }
    let callback = match result.reaction {
        Reaction::None => None,
        Reaction::Pain => target.traits.pain,
        Reaction::Death => target.traits.die,
    };
    if let Some(callback) = callback {
        dispatch(callback, target, event, result);
    }
    result
}

#[derive(Clone, Copy, Debug)]
pub struct Blast {
    pub origin: Vec3,
    pub amount: f32,
    pub radius: f32,
    pub attacker: Option<EntityId>,
    pub inflictor: Option<EntityId>,
    pub ignore: Option<EntityId>,
}

#[derive(Clone, Copy, Debug)]
pub struct BlastTarget {
    pub id: EntityId,
    pub body: Body,
    pub damageable: bool,
    /// Authored character response, such as the Q1 shambler's explosion resistance.
    pub multiplier: f32,
}

/// Candidates come from the shared area index; visibility uses the shared trace.
/// Falloff belongs to the attack, while emit resolves each victim's DamageRules.
pub fn radius_damage(
    rules: &DamageRules,
    blast: Blast,
    targets: impl IntoIterator<Item = BlastTarget>,
    mut can_damage: impl FnMut(EntityId, Vec3) -> bool,
    mut emit: impl FnMut(DamageEvent),
) {
    for target in targets {
        if !target.damageable || Some(target.id) == blast.ignore {
            continue;
        }
        let Some((amount, direction)) = (rules.radius)(blast, target) else {
            continue;
        };
        if can_damage(target.id, blast.origin) {
            emit(DamageEvent {
                target: target.id,
                attacker: blast.attacker,
                inflictor: blast.inflictor,
                amount,
                knockback: amount as i32,
                direction: Some(direction),
                point: blast.origin,
                flags: DamageFlags(DamageFlags::RADIUS),
            });
        }
    }
}

pub(crate) fn impulse(target: &mut DamageTarget<'_>, direction: Vec3, force: f32) {
    for axis in 0..3 {
        target.velocity.0[axis] += direction.0[axis] * force;
    }
}

pub(crate) fn center(body: Body) -> Vec3 {
    Vec3(std::array::from_fn(|axis| {
        body.position.0[axis] + (body.mins.0[axis] + body.maxs.0[axis]) * 0.5
    }))
}

pub(crate) fn center_falloff(blast: Blast, target: BlastTarget, radius: f32) -> Option<f32> {
    let distance = length(difference(blast.origin, center(target.body)));
    if distance > radius {
        return None;
    }
    let mut points = blast.amount - distance * 0.5;
    if Some(target.id) == blast.attacker {
        points *= 0.5;
    }
    points *= target.multiplier;
    (points > 0.0).then_some(points)
}
