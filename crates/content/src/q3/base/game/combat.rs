//! Quake III base/game: combat.
//!
//! Donor provenance: `src/content/q3/base/game/combat.ts`.

use qa_core::identity::ActorId;
use qa_core::math::{add3, length3, normalize3, scale3, sub3, vec3, Bounds, Vec3};
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::entities::{
    use_actor, DamageParticipant, EntityRef, ItemId, PoolHandle, ProviderId, Q3ItemTable,
};
use crate::q3::base::game::radius_damage::Q3RadiusTarget;
use crate::q3::base::game::state::{GameFlags, MoverState};
use crate::q3::base::shared::definitions::{
    stat_schema, EntityEvent, EntityType, GameType, PersistentIndex, Powerup, Product, StatSchema, ARMOR_PROTECTION,
};
use crate::q3::base::shared::player_state::{MoveFlags, ENTITYNUM_NONE, ENTITYNUM_WORLD};
use crate::q3::base::world::{ActorTraceHit, ActorTraceQuery, ActorTraceResult, TraceShape};

// ---------------------------------------------------------------------------
// Combat (combat.ts).
// ---------------------------------------------------------------------------

/// Juiced means of death (`MOD_JUICED`).
pub const MOD_JUICED: i32 = 27;

/// Damage flags (`DamageFlags`).
pub struct DamageFlags;

impl DamageFlags {
    /// Radius damage.
    pub const RADIUS: i32 = 0x1;
    /// Bypass armor.
    pub const NO_ARMOR: i32 = 0x2;
    /// No knockback.
    pub const NO_KNOCKBACK: i32 = 0x4;
    /// No protection.
    pub const NO_PROTECTION: i32 = 0x8;
    /// Bypass team protection.
    pub const NO_TEAM_PROTECTION: i32 = 0x10;
}

/// Damage diagnostic record (`DamageDiagnostic`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageDiagnostic {
    /// Time milliseconds.
    pub time: i32,
    /// Entity number.
    pub entity_num: i32,
    /// Health before.
    pub health: i32,
    /// Applied damage.
    pub damage: i32,
    /// Armor saved.
    pub armor: i32,
}

/// Synchronous damage application helper.
pub type DamageApply<'a> = &'a dyn Fn() -> DamageOutcome;

/// Actor observations (`CombatServices.actors`).
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct CombatActors {
    /// Actor is live (`isLive`).
    pub is_live: Rc<dyn Fn(&ActorId) -> bool>,
    /// Participant by actor (`participant`).
    pub participant: Rc<dyn Fn(&ActorId) -> DamageParticipant>,
    /// Parent actor (`parent`).
    pub parent: Rc<dyn Fn(&ActorId) -> Option<ActorId>>,
    /// Linked bounds (`linkedBounds`).
    pub linked_bounds: Rc<dyn Fn(&ActorId) -> Option<Bounds>>,
    /// Actor is a player (`isPlayer`).
    pub is_player: Rc<dyn Fn(&ActorId) -> bool>,
}

/// Combat product services (`CombatContext` product union).
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub enum CombatProduct {
    /// Base game.
    Baseq3,
    /// Mission pack gates.
    Missionpack {
        /// Obelisk attack interceptor.
        check_obelisk_attack: Rc<dyn Fn(&EntityRef, &DamageParticipant) -> bool>,
        /// Invulnerability impact effect.
        invulnerability_effect: Rc<dyn Fn(&EntityRef, Vec3, Vec3)>,
    },
}

/// Combat services and rules (`CombatContext`).
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct CombatContext {
    /// Gameplay authority.
    pub authority: CombatAuthority,
    /// Source damage modifier.
    pub source_damage_modifier: Option<SourceDamageModifier>,
    /// Attack provenance builder.
    pub attack:
        Rc<dyn Fn(DamageParticipant, DamageParticipant, Option<ItemId>, i32, i32, Option<ActorId>) -> AttackProvenance>,
    /// Damage dispatcher.
    pub dispatch: Rc<dyn for<'a> Fn(Q3DamageCall, DamageApply<'a>) -> DamageOutcome>,
    /// Current time milliseconds.
    pub time: i32,
    /// Intermission queued flag.
    pub intermission_queued: i32,
    /// Game type.
    pub game_type: i32,
    /// Friendly fire enabled.
    pub friendly_fire: bool,
    /// Knockback scale.
    pub knockback: f32,
    /// Entity pool.
    pub entities: PoolHandle,
    /// Spatial queries.
    pub spatial: SpatialQueries,
    /// Actor observations.
    pub actors: CombatActors,
    /// Damage debugger.
    pub debug_damage: Option<Rc<dyn Fn(DamageDiagnostic)>>,
    /// Carrier-hurt hook.
    pub check_hurt_carrier: Rc<dyn Fn(&EntityRef, &EntityRef)>,
    /// Accuracy-hit logger.
    pub log_accuracy_hit: Rc<dyn Fn(&EntityRef, &EntityRef) -> bool>,
    /// Product services.
    pub product: CombatProduct,
    /// Item table.
    pub item_table: Rc<Q3ItemTable>,
}

/// Native damage call record (`Q3DamageCall`).
#[derive(Clone)]
pub struct Q3DamageCall {
    /// Target entity.
    pub target: EntityRef,
    /// Inflictor participant.
    pub source: DamageParticipant,
    /// Attacker participant.
    pub owner: DamageParticipant,
    /// Impulse direction.
    pub direction: Option<Vec3>,
    /// Impact point.
    pub point: Option<Vec3>,
    /// Damage amount.
    pub amount: f32,
    /// Damage flags.
    pub flags: i32,
    /// Means of death.
    pub method_of_death: i32,
}

/// Target admission decision (`q3AdmitTargetDamage` result).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmitDecision {
    /// Continue with damage.
    Continue,
    /// Damage handled.
    Handled,
}

/// Mission-pack invulnerability gate (`q3InvulnerabilityBlocks`).
#[must_use]
pub fn q3_invulnerability_blocks(
    context: &CombatContext,
    target: &EntityRef,
    direction: Option<Vec3>,
    point: Option<Vec3>,
    method_of_death: i32,
) -> bool {
    let CombatProduct::Missionpack {
        invulnerability_effect, ..
    } = &context.product
    else {
        return false;
    };
    let borrowed = target.borrow();
    let Some(client) = borrowed.client.as_ref() else {
        return false;
    };
    if method_of_death == MOD_JUICED || client.invulnerability_time <= context.time {
        return false;
    }
    drop(borrowed);
    if let (Some(direction), Some(point)) = (direction, point) {
        invulnerability_effect(target, direction, point);
    }
    true
}

/// Same-team test (`onSameTeam`).
pub(crate) fn on_same_team(context: &CombatContext, first: &EntityRef, second: &EntityRef) -> bool {
    let first = first.borrow();
    let second = second.borrow();
    match (first.client.as_ref(), second.client.as_ref()) {
        (Some(a), Some(b)) => {
            context.game_type >= GameType::GtTeam as i32 && a.sess.session_team == b.sess.session_team
        }
        _ => false,
    }
}

/// Ranked means of death shared by combat and death reports.
pub(crate) fn ranked_means_of_death(product: Product, method_of_death: i32) -> i32 {
    if product == Product::Missionpack && method_of_death >= 23 {
        if method_of_death == 28 {
            23
        } else {
            0
        }
    } else {
        method_of_death
    }
}

/// Product of a combat context.
pub(crate) fn combat_product(context: &CombatContext) -> Product {
    match context.product {
        CombatProduct::Baseq3 => Product::Baseq3,
        CombatProduct::Missionpack { .. } => Product::Missionpack,
    }
}

/// Armor absorption (`checkArmor`, `CheckArmor`).
pub fn check_armor(target: &EntityRef, damage: f32, flags: i32) -> i32 {
    if damage == 0.0 {
        return 0;
    }
    if target.borrow().client.is_none() || (flags & DamageFlags::NO_ARMOR) != 0 {
        return 0;
    }
    let product = target
        .borrow()
        .client
        .as_ref()
        .map_or(Product::Baseq3, |client| client.ps.product);
    let slot = match stat_schema(product) {
        StatSchema::Base(layout) => layout.armor,
        StatSchema::Missionpack(layout) => layout.armor,
    };
    let scaled = damage * ARMOR_PROTECTION as f32;
    let rounded_save = scaled.ceil() as i32;
    let mut borrowed = target.borrow_mut();
    let Some(client) = borrowed.client.as_mut() else {
        return 0;
    };
    let armor = client.ps.stats.get(slot);
    let save = if rounded_save >= armor { armor } else { rounded_save };
    if save == 0 {
        return 0;
    }
    client.ps.stats.set(slot, armor - save);
    save
}

/// Target admission gate (`q3AdmitTargetDamage`).
pub fn q3_admit_target_damage(
    context: &CombatContext,
    target: &EntityRef,
    source: &DamageParticipant,
    owner: &DamageParticipant,
) -> AdmitDecision {
    {
        let borrowed = target.borrow();
        if !borrowed.takedamage || context.intermission_queued != 0 {
            return AdmitDecision::Handled;
        }
        if borrowed.s.e_type != EntityType::EtMover as i32 {
            return AdmitDecision::Continue;
        }
        if borrowed.use_callback.is_none() || borrowed.mover_state != MoverState::Pos1 {
            return AdmitDecision::Handled;
        }
    }
    let use_callback = target.borrow().use_callback.clone();
    if let Some(use_callback) = use_callback {
        use_callback(target.clone(), Some(source.clone()), Some(owner.clone()));
    }
    AdmitDecision::Handled
}

/// Apply damage (`damage`, `G_Damage`).
#[allow(clippy::too_many_arguments)]
pub fn damage(
    context: &CombatContext,
    target: DamageParticipant,
    inflictor: Option<DamageParticipant>,
    attacker: Option<DamageParticipant>,
    mut direction: Option<&mut Vec3>,
    point: Option<Vec3>,
    amount: f32,
    flags: i32,
    method_of_death: i32,
    originating_projectile: Option<ActorId>,
) {
    if context.intermission_queued != 0 {
        return;
    }
    let mut flags = flags;
    let DamageParticipant::Native(entity) = &target else {
        let origin = match (&target, point) {
            (_, Some(point)) => Some(point),
            (DamageParticipant::Shared(shared), None) => shared.origin,
            _ => None,
        };
        let Some(origin) = origin else {
            return;
        };
        let world = context.entities.borrow().at(ENTITYNUM_WORLD);
        let source = inflictor.unwrap_or(DamageParticipant::Native(world.clone()));
        let owner = attacker.unwrap_or(DamageParticipant::Native(world));
        let impulse = direction.as_deref().copied().unwrap_or(vec3(0.0, 0.0, 0.0));
        if let Some(direction) = direction.as_deref_mut() {
            *direction = normalize3(*direction);
        } else {
            flags |= DamageFlags::NO_KNOCKBACK;
        }
        let attack = (context.attack)(source, owner, None, method_of_death, flags, originating_projectile);
        let request = DamageRequest {
            attack,
            target: use_actor(&target),
            amount,
            knockback: amount,
            direction: impulse,
            point: origin,
            normal: vec3(0.0, 0.0, 0.0),
            delivery: if (flags & DamageFlags::RADIUS) != 0 {
                DamageDelivery::Radius
            } else {
                DamageDelivery::Direct
            },
        };
        let actors = context.actors.clone();
        (context.authority.apply)(apply_source_damage_modifier(
            &request,
            context.source_damage_modifier.as_ref(),
            &|actor| (actors.is_live)(actor),
        ));
        return;
    };
    if !entity.borrow().takedamage {
        return;
    }
    let preview = direction.as_deref().copied();
    if q3_invulnerability_blocks(context, entity, preview, point, method_of_death) {
        return;
    }
    let world = context.entities.borrow().at(ENTITYNUM_WORLD);
    let source = inflictor.unwrap_or(DamageParticipant::Native(world.clone()));
    let owner = attacker.unwrap_or(DamageParticipant::Native(world));
    if q3_admit_target_damage(context, entity, &source, &owner) == AdmitDecision::Handled {
        return;
    }
    if matches!(context.product, CombatProduct::Missionpack { .. }) && context.game_type == GameType::GtObelisk as i32 {
        let CombatProduct::Missionpack {
            check_obelisk_attack, ..
        } = &context.product
        else {
            return;
        };
        if check_obelisk_attack(entity, &owner) {
            return;
        }
    }
    if entity.borrow().client.as_ref().is_some_and(|client| client.noclip) {
        return;
    }
    let impulse = direction.as_deref().copied().unwrap_or(vec3(0.0, 0.0, 0.0));
    let call_direction = direction.as_deref().copied().map(normalize3);
    if let Some(direction) = direction {
        *direction = normalize3(*direction);
    } else {
        flags |= DamageFlags::NO_KNOCKBACK;
    }
    let attack = (context.attack)(
        source.clone(),
        owner.clone(),
        None,
        method_of_death,
        flags,
        originating_projectile,
    );
    let call = Q3DamageCall {
        target: entity.clone(),
        source,
        owner,
        direction: call_direction,
        point,
        amount,
        flags,
        method_of_death,
    };
    let origin = point.unwrap_or_else(|| entity.borrow().r.current_origin);
    let target_actor = entity.borrow().actor.id.clone();
    let authority = context.authority.clone();
    let modifier = context.source_damage_modifier.clone();
    let actors = context.actors.clone();
    let apply = || {
        (authority.apply)(apply_source_damage_modifier(
            &DamageRequest {
                attack: attack.clone(),
                target: target_actor.clone(),
                amount,
                knockback: amount,
                direction: impulse,
                point: origin,
                normal: vec3(0.0, 0.0, 0.0),
                delivery: if (flags & DamageFlags::RADIUS) != 0 {
                    DamageDelivery::Radius
                } else {
                    DamageDelivery::Direct
                },
            },
            modifier.as_ref(),
            &|actor| (actors.is_live)(actor),
        ))
    };
    (context.dispatch)(call, &apply);
}

/// Regular armor points when present.
pub(crate) fn regular_points(armor: &RegularArmor) -> Option<i32> {
    match armor {
        RegularArmor::None => None,
        RegularArmor::Q1 { points, .. }
        | RegularArmor::Q2 { points, .. }
        | RegularArmor::Q3 { points, .. }
        | RegularArmor::Source { points, .. } => Some(*points),
    }
}

/// Native entities share identity by handle.
pub(crate) fn same_native(left: &EntityRef, right: &EntityRef) -> bool {
    Rc::ptr_eq(left, right)
}

/// Participant-to-entity identity.
pub(crate) fn participant_is_entity(participant: &DamageParticipant, entity: &EntityRef) -> bool {
    match participant {
        DamageParticipant::Native(other) => same_native(other, entity),
        DamageParticipant::Shared(_) => false,
    }
}

/// Damage feedback after authority commits (`q3DamageFeedback`).
pub fn q3_damage_feedback(context: &CombatContext, call: &Q3DamageCall, decision: &DamageDecision) {
    let native_owner = match &call.owner {
        DamageParticipant::Native(entity) => Some(entity.clone()),
        DamageParticipant::Shared(_) => None,
    };
    let product = combat_product(context);
    let mut incoming = call.amount.trunc() as i32;
    if let Some(owner) = &native_owner {
        let different = !same_native(owner, &call.target);
        let has_client = owner.borrow().client.is_some();
        if has_client && different {
            let (max_health_slot, persistent_powerup) = match stat_schema(product) {
                StatSchema::Base(layout) => (layout.max_health, None),
                StatSchema::Missionpack(layout) => (layout.max_health, Some(layout.persistent_powerup)),
            };
            let maximum = owner
                .borrow()
                .client
                .as_ref()
                .map_or(0, |client| client.ps.stats.get(max_health_slot));
            let mut maximum = maximum;
            if product == Product::Missionpack {
                if let Some(persistent) = persistent_powerup {
                    let index = owner
                        .borrow()
                        .client
                        .as_ref()
                        .map_or(0, |client| client.ps.stats.get(persistent));
                    if context.item_table.item_at(index).tag == Powerup::PwGuard as i32 {
                        maximum /= 2;
                    }
                }
            }
            incoming = incoming.wrapping_mul(maximum) / 100;
        }
    }
    let target_flags = call.target.borrow().flags;
    let knockback = if (call.flags & DamageFlags::NO_KNOCKBACK) != 0 || (target_flags & GameFlags::NO_KNOCKBACK) != 0 {
        0
    } else {
        incoming.min(200)
    };
    if knockback != 0 && call.direction.is_some() {
        let mut borrowed = call.target.borrow_mut();
        if let Some(client) = borrowed.client.as_mut() {
            if client.ps.pm_time == 0 {
                client.ps.pm_time = 200.min(50.max(knockback.wrapping_mul(2)));
                client.ps.pm_flags |= MoveFlags::TimeKnockback as i32;
            }
        }
    }
    if (call.flags & DamageFlags::NO_PROTECTION) == 0 {
        let check_team = product == Product::Baseq3
            || (call.method_of_death != MOD_JUICED && (call.flags & DamageFlags::NO_TEAM_PROTECTION) == 0);
        if check_team
            && !participant_is_entity(&call.owner, &call.target)
            && native_owner
                .as_ref()
                .is_some_and(|owner| on_same_team(context, &call.target, owner) && !context.friendly_fire)
        {
            return;
        }
        let parent_entity = if product == Product::Missionpack && call.method_of_death == 25 {
            (context.actors.parent)(&use_actor(&call.source))
                .and_then(|parent| context.entities.borrow().native_by_actor(&parent))
        } else {
            None
        };
        if product == Product::Missionpack
            && call.method_of_death == 25
            && (participant_is_entity(&call.owner, &call.target)
                || parent_entity
                    .as_ref()
                    .is_some_and(|parent| on_same_team(context, &call.target, parent)))
        {
            return;
        }
        let target_actor = call.target.borrow().actor.id.clone();
        if (call.target.borrow().flags & GameFlags::GODMODE) != 0
            || (context.authority.read)(&target_actor)
                .as_ref()
                .is_some_and(|state| state.invulnerable)
        {
            return;
        }
    }
    let battlesuit = call
        .target
        .borrow()
        .client
        .as_ref()
        .is_some_and(|client| client.ps.powerups.get(Powerup::PwBattlesuit as i32) != 0);
    if battlesuit {
        context
            .entities
            .borrow()
            .add_event(&call.target, EntityEvent::EvPowerupBattlesuit as i32, 0);
        if (call.flags & DamageFlags::RADIUS) != 0 || call.method_of_death == 19 {
            return;
        }
    }
    q3_committed_damage_feedback(
        context,
        &call.target,
        native_owner.as_ref(),
        call.direction,
        knockback,
        call.method_of_death,
        decision,
    );
}

/// Foreign-attacker damage feedback (`q3ForeignDamageFeedback`).
pub fn q3_foreign_damage_feedback(
    context: &CombatContext,
    target: &EntityRef,
    owner: Option<&EntityRef>,
    decision: &DamageDecision,
) {
    let Some(DamageFeedback::Q3 { knockback, battlesuit }) = decision.feedback else {
        return;
    };
    let incoming = decision.request.direction;
    let direction = if incoming.x == 0.0 && incoming.y == 0.0 && incoming.z == 0.0 {
        None
    } else {
        Some(normalize3(incoming))
    };
    if knockback != 0 && direction.is_some() {
        let mut borrowed = target.borrow_mut();
        if let Some(client) = borrowed.client.as_mut() {
            if client.ps.pm_time == 0 {
                client.ps.pm_time = 200.min(50.max(knockback.wrapping_mul(2)));
                client.ps.pm_flags |= MoveFlags::TimeKnockback as i32;
            }
        }
    }
    if battlesuit {
        context
            .entities
            .borrow()
            .add_event(target, EntityEvent::EvPowerupBattlesuit as i32, 0);
    }
    let method_of_death = match &decision.request.attack.cause {
        AttackCause::Q3 { means_of_death, .. } => *means_of_death,
        _ => 0,
    };
    q3_committed_damage_feedback(context, target, owner, direction, knockback, method_of_death, decision);
}

/// Committed damage bookkeeping (`q3CommittedDamageFeedback`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn q3_committed_damage_feedback(
    context: &CombatContext,
    target: &EntityRef,
    native_owner: Option<&EntityRef>,
    direction: Option<Vec3>,
    knockback: i32,
    method_of_death: i32,
    decision: &DamageDecision,
) {
    if decision.applied_damage == 0
        && !decision
            .mutations
            .iter()
            .any(|mutation| matches!(mutation, DamageMutation::Armor { .. }))
    {
        return;
    }
    let mut armor = 0;
    for mutation in &decision.mutations {
        if let DamageMutation::Armor { before, after } = mutation {
            if let (Some(was), Some(now)) = (regular_points(&before.regular), regular_points(&after.regular)) {
                armor += was - now;
            }
        }
    }
    let product = combat_product(context);
    let has_client = target.borrow().client.is_some();
    if has_client {
        let victim = target.borrow().slot as i32;
        let attacker = native_owner.map_or(ENTITYNUM_WORLD, |owner| owner.borrow().s.number);
        let same_team = native_owner
            .as_ref()
            .is_some_and(|owner| on_same_team(context, target, owner));
        let owner_is_client = native_owner
            .as_ref()
            .is_some_and(|owner| owner.borrow().client.is_some());
        context.entities.borrow().rankings.borrow().damage(
            victim,
            attacker,
            decision.applied_damage + armor,
            ranked_means_of_death(product, method_of_death),
            context.time,
            owner_is_client,
            same_team,
        );
    }
    let previous_health = decision
        .mutations
        .iter()
        .find_map(|mutation| match mutation {
            DamageMutation::Health { before, .. } => Some(*before),
            _ => None,
        })
        .unwrap_or_else(|| target.borrow().health);
    if let Some(owner) = native_owner {
        let different = !same_native(owner, target);
        let owner_has_client = owner.borrow().client.is_some();
        let target_type = target.borrow().s.e_type;
        if owner_has_client
            && different
            && previous_health > 0
            && target_type != EntityType::EtMissile as i32
            && target_type != EntityType::EtGeneral as i32
        {
            let delta = if on_same_team(context, target, owner) { -1 } else { 1 };
            let mut borrowed = owner.borrow_mut();
            if let Some(client) = borrowed.client.as_mut() {
                let hits = client.ps.persistant.get(PersistentIndex::PersHits as i32);
                client.ps.persistant.set(PersistentIndex::PersHits as i32, hits + delta);
                let previous_armor = target.borrow().client.as_ref().map_or(0, |client| {
                    let armor_slot = match stat_schema(product) {
                        StatSchema::Base(layout) => layout.armor,
                        StatSchema::Missionpack(layout) => layout.armor,
                    };
                    client.ps.stats.get(armor_slot)
                }) + armor;
                client.ps.persistant.set(
                    PersistentIndex::PersAttackeeArmor as i32,
                    (previous_health << 8) | previous_armor,
                );
            }
        }
    }
    if let Some(debug) = &context.debug_damage {
        debug(DamageDiagnostic {
            time: context.time,
            entity_num: target.borrow().s.number,
            health: previous_health,
            damage: decision.applied_damage,
            armor,
        });
    }
    {
        let owner_number = native_owner.map_or(ENTITYNUM_NONE, |owner| owner.borrow().s.number);
        let mut borrowed = target.borrow_mut();
        let current_origin = borrowed.r.current_origin;
        if let Some(client) = borrowed.client.as_mut() {
            client
                .ps
                .persistant
                .set(PersistentIndex::PersAttacker as i32, owner_number);
            client.damage_armor = client.damage_armor.wrapping_add(armor);
            client.damage_blood = client.damage_blood.wrapping_add(decision.applied_damage);
            client.damage_knockback = client.damage_knockback.wrapping_add(knockback);
            client.damage_from = direction.unwrap_or(current_origin);
            client.damage_from_world = direction.is_none();
            client.last_hurt_client = owner_number;
            client.last_hurt_mod = method_of_death;
        }
    }
    if let Some(owner) = native_owner {
        if context.game_type == GameType::GtCtf as i32
            || (product == Product::Missionpack && context.game_type == GameType::Gt1fctf as i32)
        {
            (context.check_hurt_carrier)(target, owner);
        }
    }
    if decision.reaction == DamageReaction::Death && has_client {
        target.borrow_mut().flags |= GameFlags::NO_KNOCKBACK;
    }
}

/// Damage visibility test (`canDamage`, `CanDamage`).
#[must_use]
pub fn can_damage(context: &CombatContext, target: &DamageParticipant, origin: Vec3) -> bool {
    let actor = use_actor(target);
    match (context.actors.linked_bounds)(&actor) {
        Some(bounds) => q3_can_damage(&context.spatial, &actor, &bounds, origin),
        None => false,
    }
}

/// Radius damage (`radiusDamage`, `G_RadiusDamage`).
#[allow(clippy::too_many_arguments)]
pub fn radius_damage(
    context: &CombatContext,
    origin: Vec3,
    attacker: &DamageParticipant,
    amount: f32,
    radius: f32,
    ignore: Option<&DamageParticipant>,
    method_of_death: i32,
    originating_projectile: Option<ActorId>,
) -> bool {
    let owner_actor = use_actor(attacker);
    let context_for_target = context.clone();
    let owner_for_target = owner_actor.clone();
    let host = Q3RadiusHost {
        spatial: context.spatial.clone(),
        target: Rc::new(move |actor: &ActorId| {
            if !(context_for_target.authority.read)(actor)
                .as_ref()
                .is_some_and(|state| state.can_take_damage)
            {
                return None;
            }
            let bounds = (context_for_target.actors.linked_bounds)(actor)?;
            let target = (context_for_target.actors.participant)(actor);
            let position = match &target {
                DamageParticipant::Native(entity) => Some(entity.borrow().r.current_origin),
                DamageParticipant::Shared(shared) => shared.origin,
            }?;
            let owner = context_for_target.entities.borrow().native_by_actor(&owner_for_target);
            let eligible = match (&target, owner) {
                (DamageParticipant::Native(entity), Some(owner)) => {
                    (context_for_target.log_accuracy_hit)(entity, &owner)
                }
                _ => false,
            };
            Some(Q3RadiusTarget {
                origin: position,
                bounds,
                accuracy_eligible: eligible,
            })
        }),
        damage: {
            let context = context.clone();
            let owner_actor = owner_actor.clone();
            Rc::new(move |actor: &ActorId, direction: Vec3, point: Vec3, points: i32| {
                let mut impulse = direction;
                damage(
                    &context,
                    (context.actors.participant)(actor),
                    None,
                    Some((context.actors.participant)(&owner_actor)),
                    Some(&mut impulse),
                    Some(point),
                    points as f32,
                    DamageFlags::RADIUS,
                    method_of_death,
                    originating_projectile.clone(),
                );
            })
        },
    };
    let ignored = ignore.map(use_actor);
    q3_radius_damage(&host, origin, amount, radius, ignored.as_ref())
}

// ---------------------------------------------------------------------------
// Combat records (unified from `mirrors_game_sim.rs`): attack provenance,
// armor, damage decisions, and radius-damage hosts.
// ---------------------------------------------------------------------------

/// Actor spatial queries (`ActorSpatialQueries`).
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct SpatialQueries {
    /// Actors overlapping bounds (`areaActors`).
    pub area_actors: Rc<dyn Fn(&Bounds, i32) -> Vec<ActorId>>,
    /// Trace against actors (`traceActor`).
    pub trace_actor: Rc<dyn Fn(&ActorTraceQuery) -> ActorTraceResult>,
}

/// Quake II native cause (`Q2NativeCause`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q2NativeCause {
    /// Classic DLL cause.
    Classic {
        /// Game.
        game: String,
        /// Native value.
        value: i32,
    },
    /// Rerelease cause.
    Rerelease {
        /// Cause id.
        id: i32,
        /// Friendly fire.
        friendly_fire: bool,
        /// No point loss.
        no_point_loss: bool,
    },
}

/// Quake I armor effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmorEffect {
    /// Bypass armor.
    Bypass,
    /// Half effectiveness.
    HalfEffectiveness,
}

/// Environment hazard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvHazard {
    /// Fall.
    Fall,
    /// Drown.
    Drown,
    /// Lava.
    Lava,
    /// Slime.
    Slime,
    /// Crush.
    Crush,
    /// Trigger.
    Trigger,
}

/// Attack cause (`AttackProvenance.cause`).
#[derive(Debug, Clone, PartialEq)]
pub enum AttackCause {
    /// Quake I cause.
    Q1 {
        /// Death type.
        death_type: String,
        /// Armor effect override.
        armor_effect: Option<ArmorEffect>,
    },
    /// Quake II cause.
    Q2 {
        /// Means of death.
        means_of_death: i32,
        /// Damage flags.
        damage_flags: i32,
        /// Native cause.
        native: Option<Q2NativeCause>,
    },
    /// Quake III cause.
    Q3 {
        /// Means of death.
        means_of_death: i32,
        /// Damage flags.
        damage_flags: i32,
    },
    /// Environment cause.
    Environment {
        /// Hazard.
        hazard: EnvHazard,
    },
}

/// Source clock time (`SourceTime`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceTime {
    /// Millisecond clock.
    Milliseconds {
        /// Millisecond value.
        value: i32,
    },
}

/// Captured attack provenance (`AttackProvenance`).
#[derive(Debug, Clone, PartialEq)]
pub struct AttackProvenance {
    /// Sequence number.
    pub sequence: i32,
    /// Source time.
    pub time: SourceTime,
    /// Attacker actor.
    pub attacker: Option<ActorId>,
    /// Inflictor actor.
    pub inflictor: Option<ActorId>,
    /// Originating projectile.
    pub originating_projectile: Option<ActorId>,
    /// Weapon item.
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
    /// Cause.
    pub cause: AttackCause,
}

/// Attacker damage policy (`SourceDamageModifier`).
#[derive(Clone)]
pub struct SourceDamageModifier {
    /// Owning provider.
    pub owner: ProviderId,
    /// Amount transform over the live attacker.
    pub transform: Rc<dyn Fn(Option<ActorId>, f32) -> f32>,
}

/// Damage delivery (`DamageRequest.delivery`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageDelivery {
    /// Direct damage.
    Direct,
    /// Radius damage.
    Radius,
}

/// Damage request (`DamageRequest`).
#[derive(Debug, Clone, PartialEq)]
pub struct DamageRequest {
    /// Attack provenance.
    pub attack: AttackProvenance,
    /// Target actor.
    pub target: ActorId,
    /// Damage amount.
    pub amount: f32,
    /// Knockback amount.
    pub knockback: f32,
    /// Impulse direction.
    pub direction: Vec3,
    /// Impact point.
    pub point: Vec3,
    /// Impact normal.
    pub normal: Vec3,
    /// Delivery.
    pub delivery: DamageDelivery,
}

/// Regular armor state (`RegularArmorState`).
#[derive(Debug, Clone, PartialEq)]
pub enum RegularArmor {
    /// No armor.
    None,
    /// Quake I armor.
    Q1 {
        /// Points.
        points: i32,
        /// Absorption.
        absorption: f32,
        /// Item.
        item: ItemId,
    },
    /// Quake II armor.
    Q2 {
        /// Points.
        points: i32,
        /// Normal protection.
        normal_protection: f32,
        /// Energy protection.
        energy_protection: f32,
        /// Item.
        item: ItemId,
    },
    /// Quake III armor.
    Q3 {
        /// Points.
        points: i32,
        /// Protection fraction.
        protection: f32,
    },
    /// Source armor.
    Source {
        /// Points.
        points: i32,
        /// Item.
        item: Option<ItemId>,
    },
}

/// Powered protection state (`PoweredProtectionState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PoweredProtection {
    /// No powered protection.
    None,
    /// Screen with cells.
    Screen {
        /// Cells.
        cells: i32,
    },
    /// Shield with cells.
    Shield {
        /// Cells.
        cells: i32,
    },
}

/// Armor state (`ArmorState`).
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorState {
    /// Regular armor.
    pub regular: RegularArmor,
    /// Powered protection.
    pub powered: PoweredProtection,
}

/// Combat state (`CombatState`).
#[derive(Debug, Clone, PartialEq)]
pub struct CombatState {
    /// Health.
    pub health: i32,
    /// Armor.
    pub armor: ArmorState,
    /// Mass.
    pub mass: f32,
    /// Can take damage.
    pub can_take_damage: bool,
    /// Invulnerable.
    pub invulnerable: bool,
    /// Immune to knockback.
    pub no_knockback: bool,
    /// Team identity.
    pub team: Option<String>,
}

/// Committed damage mutation (`DamageMutation`).
#[derive(Debug, Clone, PartialEq)]
pub enum DamageMutation {
    /// Health change.
    Health {
        /// Health before.
        before: i32,
        /// Health after.
        after: i32,
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
    /// Impulse.
    Impulse {
        /// Impulse vector.
        impulse: Vec3,
        /// Movement provider.
        movement_provider: ProviderId,
    },
}

/// Damage reaction (`DamageDecision.reaction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageReaction {
    /// No reaction.
    None,
    /// Pain.
    Pain,
    /// Death.
    Death,
}

/// Source damage feedback (`DamageDecision.feedback`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageFeedback {
    /// Quake II feedback.
    Q2 {
        /// Power armor saved.
        power_armor: i32,
        /// Armor saved.
        armor: i32,
        /// Blood damage.
        blood: i32,
        /// Knockback.
        knockback: i32,
    },
    /// Quake III feedback.
    Q3 {
        /// Knockback.
        knockback: i32,
        /// Battlesuit absorbed.
        battlesuit: bool,
    },
}

/// Committed damage decision (`DamageDecision`).
#[derive(Debug, Clone, PartialEq)]
pub struct DamageDecision {
    /// Request.
    pub request: DamageRequest,
    /// Committed mutations.
    pub mutations: Vec<DamageMutation>,
    /// Applied health damage.
    pub applied_damage: i32,
    /// Reaction.
    pub reaction: DamageReaction,
    /// Source feedback.
    pub feedback: Option<DamageFeedback>,
}

/// Damage outcome (`DamageOutcome`).
#[derive(Debug, Clone, PartialEq)]
pub enum DamageOutcome {
    /// Target went stale.
    StaleTarget {
        /// Request.
        request: DamageRequest,
    },
    /// Committed decision.
    Committed {
        /// Decision.
        decision: DamageDecision,
        /// Target survived.
        survived: bool,
    },
}

/// Gameplay authority operations (`GameplayAuthority`).
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct CombatAuthority {
    /// Apply a damage request (`apply`).
    pub apply: Rc<dyn Fn(DamageRequest) -> DamageOutcome>,
    /// Read combat state (`read`).
    pub read: Rc<dyn Fn(&ActorId) -> Option<CombatState>>,
}

/// Apply the source damage modifier (`applySourceDamageModifier`).
#[must_use]
pub fn apply_source_damage_modifier(
    request: &DamageRequest,
    modifier: Option<&SourceDamageModifier>,
    is_live: &dyn Fn(&ActorId) -> bool,
) -> DamageRequest {
    let Some(modifier) = modifier else {
        return request.clone();
    };
    let current =
        |actor: &Option<ActorId>| -> Option<ActorId> { actor.as_ref().filter(|handle| is_live(handle)).cloned() };
    let mut attack = request.attack.clone();
    attack.attacker = current(&request.attack.attacker);
    attack.inflictor = current(&request.attack.inflictor);
    let amount = if attack.damage_powerup_owner.as_ref() == Some(&modifier.owner) {
        request.amount
    } else {
        (modifier.transform)(attack.attacker.clone(), request.amount)
    };
    attack.damage_powerup_owner = Some(modifier.owner.clone());
    DamageRequest {
        attack,
        amount,
        ..request.clone()
    }
}

// ---------------------------------------------------------------------------
// Radius damage (mirror of `game/radius-damage.ts`).
// ---------------------------------------------------------------------------

/// Radius-damage host (`Q3RadiusHost`).
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct Q3RadiusHost {
    /// Spatial queries.
    pub spatial: SpatialQueries,
    /// Target record by actor.
    pub target: Rc<dyn Fn(&ActorId) -> Option<Q3RadiusTarget>>,
    /// Damage application.
    pub damage: Rc<dyn Fn(&ActorId, Vec3, Vec3, i32)>,
}

/// Visibility check for radius damage (`q3CanDamage`).
#[must_use]
pub fn q3_can_damage(spatial: &SpatialQueries, actor: &ActorId, bounds: &Bounds, origin: Vec3) -> bool {
    let midpoint = scale3(add3(bounds.min, bounds.max), 0.5);
    let trace = |end: Vec3| -> ActorTraceResult {
        (spatial.trace_actor)(&ActorTraceQuery {
            start: origin,
            end,
            shape: TraceShape::Point,
            pass_actor: None,
            mask: 1,
        })
    };
    let center = trace(midpoint);
    if center.fraction == 1.0 {
        return true;
    }
    if let ActorTraceHit::Actor { actor: hit } = &center.hit {
        if hit == actor {
            return true;
        }
    }
    for (x, y) in [(15.0, 15.0), (15.0, -15.0), (-15.0, 15.0), (-15.0, -15.0)] {
        let end = vec3(midpoint.x + x, midpoint.y + y, midpoint.z);
        if trace(end).fraction == 1.0 {
            return true;
        }
    }
    false
}

/// Radius falloff damage over shared actors (`q3RadiusDamage`).
pub fn q3_radius_damage(host: &Q3RadiusHost, origin: Vec3, amount: f32, radius: f32, ignore: Option<&ActorId>) -> bool {
    let radius = radius.max(1.0);
    let extent = vec3(radius, radius, radius);
    let candidates = (host.spatial.area_actors)(
        &Bounds {
            min: sub3(origin, extent),
            max: add3(origin, extent),
        },
        1024,
    );
    let mut hit_client = false;
    for actor in &candidates {
        if ignore.is_some_and(|ignored| ignored == actor) {
            continue;
        }
        let Some(target) = (host.target)(actor) else {
            continue;
        };
        let axis = |value: f32, min: f32, max: f32| -> f32 {
            if value < min {
                min - value
            } else if value > max {
                value - max
            } else {
                0.0
            }
        };
        let distance = length3(vec3(
            axis(origin.x, target.bounds.min.x, target.bounds.max.x),
            axis(origin.y, target.bounds.min.y, target.bounds.max.y),
            axis(origin.z, target.bounds.min.z, target.bounds.max.z),
        ));
        if distance >= radius {
            continue;
        }
        let points = amount * (1.0 - distance / radius);
        if !q3_can_damage(&host.spatial, actor, &target.bounds, origin) {
            continue;
        }
        if target.accuracy_eligible {
            hit_client = true;
        }
        (host.damage)(
            actor,
            add3(sub3(target.origin, origin), vec3(0.0, 0.0, 24.0)),
            origin,
            points.trunc() as i32,
        );
    }
    hit_client
}
