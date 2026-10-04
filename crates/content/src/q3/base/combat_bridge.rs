//! Quake III base: combat bridge.
//!
//! Donor provenance: `src/content/q3/base/combat-bridge.ts`.

use crate::value::{int, obj, SaveJson, SaveReader, ValueError};
use qa_core::identity::{ActorId, ProviderId};
use qa_core::math::{vec3, Bounds, Vec3};
use qa_core::time::SourceTime;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::contract::{decode_attack_damage_flags, ItemId, ProtectionChannel};
use qa_world::combat::{Delivery, Reaction};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::combat::DamageDiagnostic;
use crate::q3::base::game::state::GameFlags;
use crate::q3::base::records::*;
use crate::q3::base::shared::definitions::*;
use crate::q3::base::shared::items::*;
use crate::q3::base::world::*;
use crate::q3::foundation::arsenal::q3_weapon_item;

// ---------------------------------------------------------------------------
// combat-bridge.ts
// ---------------------------------------------------------------------------

/// Combat bridge host services (`Q3CombatBridgeHost`).
///
/// Missionpack-only hooks keep defaults that baseq3 never calls, matching
/// the donor's product-discriminated union.
pub trait Q3CombatBridgeHost {
    /// Gameplay authority.
    fn authority(&self) -> Rc<dyn Q3SessionCombat>;
    /// Entity pool.
    fn entities(&self) -> EntityPoolRef;
    /// Entity records.
    fn records(&self) -> Q3EntityRecords;
    /// Server world with actor queries.
    fn world(&self) -> Rc<dyn Q3ServerWorld>;
    /// Weapon provider.
    fn weapon_provider(&self) -> ProviderId;
    /// Damage powerup owner override.
    fn damage_powerup_owner(&self) -> Option<ProviderId> {
        None
    }
    /// Source damage modifier.
    fn source_damage_modifier(&self) -> Option<SourceDamageModifier> {
        None
    }
    /// Combat provider.
    fn combat_provider(&self) -> ProviderId;
    /// Inventory provider.
    fn inventory_provider(&self) -> ProviderId;
    /// Movement provider.
    fn movement_provider(&self) -> ProviderId;
    /// Victim armor context.
    fn armor_context(&self, request: &DamageRequest) -> VictimArmorContext;
    /// Current time.
    fn time(&self) -> i32;
    /// Queued intermission.
    fn intermission_queued(&self) -> i32;
    /// Game type tag.
    fn game_type(&self) -> i32;
    /// Friendly fire.
    fn friendly_fire(&self) -> bool;
    /// Knockback scale.
    fn knockback(&self) -> f32;
    /// Product.
    fn product(&self) -> Product;
    /// Damage debug sink.
    fn debug_damage(&self, _diagnostic: &DamageDiagnostic) {}
    /// Carrier hurt hook.
    fn check_hurt_carrier(&self, target: EntityRef, attacker: EntityRef);
    /// Accuracy hit hook.
    fn log_accuracy_hit(&self, target: EntityRef, attacker: EntityRef) -> bool;
    /// Source damage feedback (`q3DamageFeedback`, game/combat.ts).
    fn damage_feedback(&self, call: &Q3DamageCall, decision: &DamageDecision);
    /// Foreign damage feedback (`q3ForeignDamageFeedback`, game/combat.ts).
    fn foreign_damage_feedback(&self, target: EntityRef, owner: Option<EntityRef>, decision: &DamageDecision);
    /// Projectile parent (missionpack only).
    fn projectile_parent(&self, _actor: &ActorId) -> Option<ActorId> {
        None
    }
    /// Obelisk attack hook (missionpack only).
    fn check_obelisk_attack(&self, _target: EntityRef, _attacker: EntityRef) -> bool {
        false
    }
    /// Invulnerability effect hook (missionpack only).
    fn invulnerability_effect(&self, _target: EntityRef, _direction: Vec3, _point: Vec3) {}
}

pub(crate) struct DamageCallGuard {
    calls: Rc<RefCell<Vec<Q3DamageCall>>>,
}

impl Drop for DamageCallGuard {
    fn drop(&mut self) {
        self.calls.borrow_mut().pop();
    }
}

/// Source combat context and feedback over the shared damage authority
/// (`Q3CombatBridge`).
#[derive(Clone)]
pub struct Q3CombatBridge {
    host: Rc<dyn Q3CombatBridgeHost>,
    context: CombatContext,
    calls: Rc<RefCell<Vec<Q3DamageCall>>>,
    sequence: Rc<Cell<i32>>,
}

impl std::fmt::Debug for Q3CombatBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Q3CombatBridge")
            .field("product", &self.context.product)
            .finish()
    }
}

pub(crate) struct ProjectedCurrent<'a> {
    bridge: Q3CombatBridge,
    request: DamageRequest,
    inner: &'a dyn CurrentCombatState,
}

impl CurrentCombatState for ProjectedCurrent<'_> {
    fn target(&self) -> Option<CombatState> {
        self.inner
            .target()
            .map(|state| self.bridge.source_state(&self.request, &state, false))
    }

    fn attacker(&self) -> Option<CombatState> {
        self.inner
            .attacker()
            .map(|state| self.bridge.source_state(&self.request, &state, true))
    }
}

impl Q3CombatBridge {
    /// Bridge over a host, building the shared combat context.
    #[must_use]
    #[allow(clippy::type_complexity)]
    pub fn new(host: Rc<dyn Q3CombatBridgeHost>) -> Self {
        let calls: Rc<RefCell<Vec<Q3DamageCall>>> = Rc::new(RefCell::new(Vec::new()));
        let sequence: Rc<Cell<i32>> = Rc::new(Cell::new(0));
        let records = host.records();
        let product = host.product();

        let actors_is_live_records = records.clone();
        let is_live: Rc<dyn Fn(&ActorId) -> bool> =
            Rc::new(move |actor| actors_is_live_records.host().actors().is_live(actor));
        let participant_records = records.clone();
        let participant: Rc<dyn Fn(&ActorId) -> DamageParticipant> =
            Rc::new(move |actor| participant_records.damage_inflictor(Some(actor)));
        let parent_host = host.clone();
        let parent: Rc<dyn Fn(&ActorId) -> Option<ActorId>> = Rc::new(move |actor| {
            if parent_host.product() == Product::Missionpack {
                parent_host.projectile_parent(actor)
            } else {
                None
            }
        });
        let bounds_records = records.clone();
        let linked_bounds: Rc<dyn Fn(&ActorId) -> Option<Bounds>> = Rc::new(move |actor| {
            bounds_records
                .host()
                .bodies()
                .linked(actor)
                .map(|linked| linked.absolute_bounds)
        });
        let player_records = records.clone();
        let is_player: Rc<dyn Fn(&ActorId) -> bool> = Rc::new(move |actor| {
            if let Some(native) = player_records.native_by_actor(Some(actor)) {
                native.borrow().client.is_some()
            } else {
                player_records.host().is_player(actor)
            }
        });

        let time_host = host.clone();
        let intermission_host = host.clone();
        let game_type_host = host.clone();
        let friendly_fire_host = host.clone();
        let knockback_host = host.clone();
        let debug_host = host.clone();
        let debug_damage: Option<Rc<dyn Fn(DamageDiagnostic)>> = Some(Rc::new(move |diagnostic| {
            debug_host.debug_damage(&diagnostic);
        }));

        let attack_host = host.clone();
        let attack_sequence = sequence.clone();
        let attack: Rc<
            dyn Fn(
                &DamageParticipant,
                &DamageParticipant,
                Option<ItemId>,
                i32,
                i32,
                Option<ActorId>,
            ) -> AttackProvenance,
        > = Rc::new(
            move |inflictor, attacker, weapon, means_of_death, flags, originating_projectile| {
                let order = attack_sequence.get();
                attack_sequence.set(order.wrapping_add(1));
                let inflictor_weapon = match inflictor {
                    DamageParticipant::Native(entity) => entity.borrow().s.weapon,
                    DamageParticipant::SharedActor(_) => 0,
                };
                let attacker_weapon = match attacker {
                    DamageParticipant::Native(entity) => entity.borrow().s.weapon,
                    DamageParticipant::SharedActor(_) => 0,
                };
                let fallback = if inflictor_weapon != 0 {
                    inflictor_weapon
                } else {
                    attacker_weapon
                };
                AttackProvenance {
                    sequence: order,
                    time: SourceTime::Milliseconds(attack_host.time()),
                    attacker: Some(use_actor(attacker)),
                    inflictor: Some(use_actor(inflictor)),
                    originating_projectile,
                    weapon: weapon.or_else(|| q3_weapon_item(fallback).map(|entry| entry.item.clone())),
                    weapon_provider: attack_host.weapon_provider(),
                    damage_powerup_owner: Some(
                        attack_host
                            .damage_powerup_owner()
                            .unwrap_or_else(|| attack_host.weapon_provider()),
                    ),
                    combat_provider: attack_host.combat_provider(),
                    inventory_provider: attack_host.inventory_provider(),
                    movement_provider: attack_host.movement_provider(),
                    cause: AttackCause::Q3 {
                        means_of_death,
                        damage_flags: flags,
                    },
                }
            },
        );

        let dispatch_calls = calls.clone();
        let dispatch: Rc<dyn Fn(Q3DamageCall, &dyn Fn() -> DamageOutcome) -> DamageOutcome> =
            Rc::new(move |call, operation| {
                dispatch_calls.borrow_mut().push(call);
                let _guard = DamageCallGuard {
                    calls: dispatch_calls.clone(),
                };
                operation()
            });

        let carrier_host = host.clone();
        let accuracy_host = host.clone();

        let (check_obelisk_attack, invulnerability_effect) = if product == Product::Missionpack {
            let obelisk_host = host.clone();
            let obelisk_records = records.clone();
            let check: Rc<dyn Fn(EntityRef, &DamageParticipant) -> bool> = Rc::new(move |target, attacker| {
                let native = match attacker {
                    DamageParticipant::Native(entity) => Some(entity.clone()),
                    DamageParticipant::SharedActor(shared) => obelisk_records.native_by_actor(Some(&shared.actor)),
                };
                if native.is_none() && obelisk_records.host().is_player(&use_actor(attacker)) {
                    panic!("Admitted Q3 map player has no native client behavior record");
                }
                native.is_some_and(|native| obelisk_host.check_obelisk_attack(target, native))
            });
            let effect_host = host.clone();
            let effect: Rc<dyn Fn(EntityRef, Vec3, Vec3)> = Rc::new(move |target, direction, point| {
                effect_host.invulnerability_effect(target, direction, point);
            });
            (Some(check), Some(effect))
        } else {
            (None, None)
        };

        let context = CombatContext {
            product,
            authority: host.authority(),
            entities: host.entities(),
            spatial: host.world(),
            source_damage_modifier: host.source_damage_modifier(),
            actors: CombatActors {
                is_live,
                participant,
                parent,
                linked_bounds,
                is_player,
            },
            time: Rc::new(move || time_host.time()),
            intermission_queued: Rc::new(move || intermission_host.intermission_queued()),
            game_type: Rc::new(move || game_type_host.game_type()),
            friendly_fire: Rc::new(move || friendly_fire_host.friendly_fire()),
            knockback: Rc::new(move || knockback_host.knockback()),
            debug_damage,
            attack,
            dispatch,
            check_hurt_carrier: Rc::new(move |target, attacker| {
                carrier_host.check_hurt_carrier(target, attacker);
            }),
            log_accuracy_hit: Rc::new(move |target, attacker| accuracy_host.log_accuracy_hit(target, attacker)),
            check_obelisk_attack,
            invulnerability_effect,
        };
        Self {
            host,
            context,
            calls,
            sequence,
        }
    }

    /// Shared combat context.
    #[must_use]
    pub fn context(&self) -> &CombatContext {
        &self.context
    }

    /// Capture the sequence save word.
    ///
    /// # Panics
    ///
    /// Panics while a combat call is retained.
    #[must_use]
    pub fn capture_save_state(&self) -> SaveJson {
        if !self.calls.borrow().is_empty() {
            panic!("Cannot save Q3 during a combat call");
        }
        obj(vec![("sequence", int(i64::from(self.sequence.get())))])
    }

    /// Restore the sequence save word.
    pub fn restore_save_state(&self, value: &SaveJson) -> Result<(), ValueError> {
        if !self.calls.borrow().is_empty() {
            panic!("Cannot restore Q3 during a combat call");
        }
        let sequence = SaveReader::at(value, "q3.combatBridge").field("sequence").integer(0)?;
        self.sequence.set(sequence as i32);
        Ok(())
    }

    /// Innermost retained source call.
    #[must_use]
    pub fn current_call(&self) -> Option<Q3DamageCall> {
        self.calls.borrow().last().cloned()
    }

    /// Shared `GameplayAuthority.beforeReaction` hook, called before
    /// dispatching actor callbacks.
    pub fn before_reaction(&self, decision: &DamageDecision) {
        let current = self.current_call();
        if let AttackCause::Q3 { .. } = decision.request.attack.cause {
            if let Some(call) = &current {
                if call.target.borrow().actor().id().clone() == decision.request.target {
                    self.host.damage_feedback(call, decision);
                    return;
                }
            }
        }
        let records = self.host.records();
        let target = records.native_by_actor(Some(&decision.request.target));
        let attacker = decision.request.attack.attacker.clone();
        let owner = attacker
            .as_ref()
            .and_then(|attacker| records.native_by_actor(Some(attacker)));
        if let Some(target) = target {
            self.host.foreign_damage_feedback(target, owner, decision);
        }
    }

    /// Combat policy for the selected provider; victims retain their own
    /// armor policy.
    #[must_use]
    pub fn policy(&self) -> CombatPolicy {
        let host = self.host.clone();
        let armor_host = host.clone();
        let armor = native_victim_armor(Rc::new(move |request| armor_host.armor_context(request)));
        let context_bridge = self.clone();
        let context = Rc::new(
            move |request: &DamageRequest, target: &CombatState, attacker: Option<&CombatState>| {
                context_bridge.policy_context(request, target, attacker)
            },
        );
        let inner = create_q3_combat_policy(host.combat_provider(), armor, context);
        let bridge = self.clone();
        CombatPolicy {
            id: inner.id.clone(),
            decide: Rc::new(move |request, target, attacker| {
                let projected_target = bridge.source_state(request, target, false);
                let projected_attacker = attacker.map(|state| bridge.source_state(request, state, true));
                let progress = (inner.decide)(request, &projected_target, projected_attacker.as_ref());
                bridge.progress(progress)
            }),
        }
    }

    fn policy_context(
        &self,
        request: &DamageRequest,
        _target: &CombatState,
        _attacker: Option<&CombatState>,
    ) -> Q3CombatContext {
        let host = &self.host;
        let records = host.records();
        let target = records.native_by_actor(Some(&request.target));
        let owner = request
            .attack
            .attacker
            .as_ref()
            .and_then(|attacker| records.native_by_actor(Some(attacker)));
        let target_client = target.as_ref().and_then(|entity| entity.borrow().client.clone());
        let owner_client = owner.as_ref().and_then(|entity| entity.borrow().client.clone());
        let method = match &request.attack.cause {
            AttackCause::Q3 { means_of_death, .. } => *means_of_death,
            _ => -1,
        };
        let parent_actor =
            if host.product() == Product::Missionpack && method == 25 && request.attack.inflictor.is_some() {
                request
                    .attack
                    .inflictor
                    .as_ref()
                    .and_then(|inflictor| (self.context.actors.parent)(inflictor))
            } else {
                None
            };
        let parent = records.native_by_actor(parent_actor.as_ref());
        let schema = stat_schema(host.product());
        let powerup_slot = match schema {
            StatSchema::Missionpack(layout) => Some(layout.persistent_powerup as usize),
            StatSchema::Base(_) => None,
        };
        let max_health = match schema {
            StatSchema::Base(layout) => layout.max_health,
            StatSchema::Missionpack(layout) => layout.max_health,
        } as usize;
        let guard = owner_client.as_ref().is_some_and(|client| {
            powerup_slot.is_some_and(|slot| {
                item_at(Product::Missionpack, client.borrow().ps.stats.get(slot))
                    .map(|item| {
                        matches!(
                            item.kind,
                            ItemKind::Powerup(Powerup::PwGuard)
                                | ItemKind::PersistantPowerup(Powerup::PwGuard)
                                | ItemKind::Team(Powerup::PwGuard)
                        )
                    })
                    .unwrap_or(false)
            })
        });
        let target_borrow = target_client.as_ref().map(|client| client.borrow());
        let owner_borrow = owner_client.as_ref().map(|client| client.borrow());
        Q3CombatContext {
            player: target_client.is_some(),
            attacker_player: owner_client.is_some(),
            attacker_max_health: owner_borrow
                .as_ref()
                .map_or(100, |client| client.ps.stats.get(max_health)),
            attacker_guard: guard,
            intermission: host.intermission_queued() != 0,
            noclip: target_borrow.as_ref().is_some_and(|client| client.noclip),
            missionpack_invulnerability: host.product() == Product::Missionpack
                && target_borrow
                    .as_ref()
                    .is_some_and(|client| client.invulnerability_time > host.time()),
            no_knockback: target
                .as_ref()
                .is_some_and(|entity| entity.borrow().flags & GameFlags::NO_KNOCKBACK != 0),
            knockback_scale: host.knockback(),
            friendly_fire: host.friendly_fire(),
            battlesuit: target_borrow
                .as_ref()
                .is_some_and(|client| client.ps.powerups.get(Powerup::PwBattlesuit as usize) != 0),
            falling: method == 19,
            juiced: method == 27,
            proximity_protected: host.product() == Product::Missionpack
                && method == 25
                && (target
                    .as_ref()
                    .is_some_and(|target| owner.as_ref().is_some_and(|owner| Rc::ptr_eq(target, owner)))
                    || parent
                        .as_ref()
                        .is_some_and(|parent| self.same_team(target.as_ref(), Some(parent)))),
            product: host.product(),
        }
    }

    fn source_state(&self, request: &DamageRequest, state: &CombatState, attacker: bool) -> CombatState {
        let host = &self.host;
        let records = host.records();
        let actor = if attacker {
            request.attack.attacker.as_ref()
        } else {
            Some(&request.target)
        };
        let entity = records.native_by_actor(actor);
        let mut next = state.clone();
        next.invulnerable = state.invulnerable
            || entity
                .as_ref()
                .is_some_and(|entity| entity.borrow().flags & GameFlags::GODMODE != 0);
        next.team = entity
            .as_ref()
            .and_then(|entity| entity.borrow().client.clone())
            .filter(|_| host.game_type() >= GameType::GtTeam as i32)
            .map(|client| format!("q3-team:{}", client.borrow().sess.session_team as i32))
            .or_else(|| state.team.clone());
        next
    }

    fn progress(&self, value: CombatProgress) -> CombatProgress {
        match value {
            CombatProgress::Complete { .. } => value,
            CombatProgress::SourceContinuation {
                request,
                mutations,
                resume,
            } => {
                let bridge = self.clone();
                CombatProgress::SourceContinuation {
                    request: request.clone(),
                    mutations,
                    resume: Rc::new(move |current| {
                        let projected = ProjectedCurrent {
                            bridge: bridge.clone(),
                            request: request.clone(),
                            inner: current,
                        };
                        bridge.progress(resume(&projected))
                    }),
                }
            }
            CombatProgress::ArmorStage {
                channel,
                request,
                mutations,
                input,
                fallback,
                resume,
            } => {
                let bridge = self.clone();
                CombatProgress::ArmorStage {
                    channel,
                    request: request.clone(),
                    mutations,
                    input,
                    fallback,
                    resume: Rc::new(move |result, current| {
                        let projected = ProjectedCurrent {
                            bridge: bridge.clone(),
                            request: request.clone(),
                            inner: current,
                        };
                        bridge.progress(resume(result, &projected))
                    }),
                }
            }
        }
    }

    fn same_team(&self, first: Option<&EntityRef>, second: Option<&EntityRef>) -> bool {
        let (Some(first), Some(second)) = (first, second) else {
            return false;
        };
        let first = first.borrow();
        let second = second.borrow();
        first.client.as_ref().is_some()
            && second.client.as_ref().is_some()
            && self.host.game_type() >= GameType::GtTeam as i32
            && first.client.as_ref().is_some_and(|client| {
                second
                    .client
                    .as_ref()
                    .is_some_and(|other| client.borrow().sess.session_team == other.borrow().sess.session_team)
            })
    }
}

// ---------------------------------------------------------------------------
// world/gameplay victim armor + Q3 policy (unified from base/mirrors.rs)
// ---------------------------------------------------------------------------

/// Armor stage word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmorStage {
    /// Power stage.
    Power,
    /// Regular stage.
    Regular,
}

/// Armor damage flags (`ArmorDamageFlags`).
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorDamageFlags {
    /// Active stage.
    pub stage: Option<ArmorStage>,
    /// Skip all armor.
    pub no_armor: bool,
    /// Skip power armor.
    pub no_power_armor: bool,
    /// Skip regular armor.
    pub no_regular_armor: bool,
    /// Energy damage.
    pub energy: bool,
    /// Regular protection scale.
    pub regular_protection_scale: Option<f32>,
}

/// Armor computation result (`ArmorResult`).
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorResult {
    /// Resulting armor.
    pub armor: ArmorState,
    /// Power damage saved.
    pub power_saved: i32,
    /// Regular damage saved.
    pub regular_saved: i32,
}

/// Armor stage input (`ArmorStageInput`).
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorStageInput {
    /// Damage request.
    pub request: DamageRequest,
    /// Direction.
    pub direction: Vec3,
    /// Point.
    pub point: Vec3,
    /// Normal.
    pub normal: Vec3,
    /// Amount.
    pub amount: i32,
    /// Flags.
    pub flags: ArmorDamageFlags,
}

/// Armor stage result (`ArmorStageResult`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmorStageResult {
    /// Damage saved.
    pub saved: i32,
}

/// Victim armor context (`VictimArmorContext`).
#[derive(Debug, Clone, PartialEq)]
pub struct VictimArmorContext {
    /// Screen facing dot.
    pub screen_facing_dot: f32,
    /// Damage arithmetic.
    pub arithmetic: VictimArithmetic,
    /// Quake II source profile.
    pub q2: Option<VictimQ2Profile>,
}

/// Victim armor arithmetic word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VictimArithmetic {
    /// Binary32.
    Binary32,
    /// Binary64.
    Binary64,
}

/// Quake II victim armor profile word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VictimQ2Profile {
    /// Classic or rerelease product.
    pub rerelease: bool,
    /// Capture-the-flag rules.
    pub ctf: bool,
    /// Victim alive.
    pub alive: bool,
}

/// Victim armor policy (`VictimArmorPolicy`).
pub type VictimArmorPolicy = Rc<dyn Fn(&DamageRequest, &ArmorState, i32, &ArmorDamageFlags) -> ArmorResult>;

/// Native victim armor over a per-request context (`nativeVictimArmor`).
#[must_use]
pub fn native_victim_armor(context: Rc<dyn Fn(&DamageRequest) -> VictimArmorContext>) -> VictimArmorPolicy {
    Rc::new(move |request, armor, damage, flags| absorb_native_armor(armor, damage, flags, &context(request)))
}

/// Native armor absorption (`absorbNativeArmor`).
///
/// # Panics
///
/// Panics when Q2 armor runs without a Q2 source profile, or when
/// source armor runs without its absorption binding.
#[must_use]
pub fn absorb_native_armor(
    armor: &ArmorState,
    damage: i32,
    flags: &ArmorDamageFlags,
    context: &VictimArmorContext,
) -> ArmorResult {
    let q2_regular = flags.stage != Some(ArmorStage::Power) && matches!(armor.regular, RegularArmorState::Q2 { .. });
    let q2_powered = flags.stage != Some(ArmorStage::Regular) && !matches!(armor.powered, PoweredProtectionState::None);
    if (q2_regular || q2_powered) && context.q2.is_none() {
        panic!("Q2 victim armor requires an explicit classic or rerelease source profile");
    }
    if damage == 0
        || flags.no_armor
        || (matches!(armor.regular, RegularArmorState::None) && matches!(armor.powered, PoweredProtectionState::None))
    {
        return ArmorResult {
            armor: armor.clone(),
            power_saved: 0,
            regular_saved: 0,
        };
    }
    let multiply = |left: f32, right: f32| -> f32 {
        if context.arithmetic == VictimArithmetic::Binary32 {
            left * right
        } else {
            (f64::from(left) * f64::from(right)) as f32
        }
    };
    let protection_scale = flags.regular_protection_scale.unwrap_or(1.0);
    let rerelease = context.q2.is_some_and(|profile| profile.rerelease);
    let facing_limit = 0.3f32;
    let mut power_saved = 0;
    let mut powered = armor.powered.clone();
    let powered_cells = match &powered {
        PoweredProtectionState::None => 0,
        PoweredProtectionState::Screen { cells } | PoweredProtectionState::Shield { cells } => *cells,
    };
    let powered_kind = match &powered {
        PoweredProtectionState::None => None,
        PoweredProtectionState::Screen { .. } => Some(0),
        PoweredProtectionState::Shield { .. } => Some(1),
    };
    if flags.stage != Some(ArmorStage::Regular)
        && !flags.no_power_armor
        && (!rerelease || context.q2.is_some_and(|profile| profile.alive))
        && powered_kind.is_some()
        && powered_cells > 0
        && (powered_kind != Some(0) || context.screen_facing_dot > facing_limit)
    {
        let is_screen = powered_kind == Some(0);
        let damage_per_cell = if is_screen || context.q2.is_some_and(|profile| profile.ctf) {
            1
        } else {
            2
        };
        // `i64` intermediates match the donor's exact float division of
        // integer words.
        let divided_damage = if is_screen {
            damage / 3
        } else {
            ((2 * i64::from(damage)) / 3) as i32
        };
        let protected_damage = if rerelease {
            divided_damage.max(1)
        } else {
            divided_damage
        };
        let doubled_cost = if rerelease {
            flags.energy
        } else {
            flags.no_regular_armor
        };
        let base_available = powered_cells * damage_per_cell;
        let divided_available = if doubled_cost {
            base_available / 2
        } else {
            base_available
        };
        let available = if rerelease {
            divided_available.max(1)
        } else {
            divided_available
        };
        power_saved = available.min(protected_damage);
        let used = (power_saved / damage_per_cell) * if doubled_cost { 2 } else { 1 };
        let remaining = if rerelease {
            0.max(powered_cells - damage_per_cell.max(used))
        } else {
            powered_cells - used
        };
        powered = match powered {
            PoweredProtectionState::Screen { .. } => PoweredProtectionState::Screen { cells: remaining },
            PoweredProtectionState::Shield { .. } => PoweredProtectionState::Shield { cells: remaining },
            PoweredProtectionState::None => PoweredProtectionState::None,
        };
    }
    let mut regular = armor.regular.clone();
    let mut regular_saved = 0;
    if !flags.no_regular_armor && flags.stage != Some(ArmorStage::Power) {
        match &regular {
            RegularArmorState::None => {}
            RegularArmorState::Source { .. } => {
                panic!("Source regular armor requires its original absorption binding");
            }
            RegularArmorState::Q1 { points, absorption, .. } => {
                let points = *points;
                let absorption = *absorption;
                regular_saved = points
                    .min(multiply(multiply(absorption, protection_scale), (damage - power_saved) as f32).ceil() as i32);
                regular = RegularArmorState::Q1 {
                    points: points - regular_saved,
                    absorption: if regular_saved >= points { 0.0 } else { absorption },
                    item: match &armor.regular {
                        RegularArmorState::Q1 { item, .. } => item.clone(),
                        _ => unreachable!("Q1 armor shape changed during absorption"),
                    },
                };
            }
            RegularArmorState::Q2 {
                points,
                normal_protection,
                energy_protection,
                ..
            } => {
                let points = *points;
                let protection = if flags.energy {
                    *energy_protection
                } else {
                    *normal_protection
                };
                regular_saved = points
                    .min(multiply(multiply(protection, protection_scale), (damage - power_saved) as f32).ceil() as i32);
                regular = match &armor.regular {
                    RegularArmorState::Q2 {
                        normal_protection,
                        energy_protection,
                        item,
                        ..
                    } => RegularArmorState::Q2 {
                        points: points - regular_saved,
                        normal_protection: *normal_protection,
                        energy_protection: *energy_protection,
                        item: item.clone(),
                    },
                    _ => unreachable!("Q2 armor shape changed during absorption"),
                };
            }
            RegularArmorState::Q3 { points, protection, .. } => {
                let points = *points;
                let protection = *protection;
                regular_saved =
                    points.min((((damage - power_saved) as f32) * (protection * protection_scale)).ceil() as i32);
                regular = RegularArmorState::Q3 {
                    points: points - regular_saved,
                    protection,
                };
            }
        }
    }
    ArmorResult {
        armor: if regular == armor.regular && powered == armor.powered {
            armor.clone()
        } else {
            ArmorState { regular, powered }
        },
        power_saved,
        regular_saved,
    }
}

/// Decoded damage flags (`attackDamageFlags`).
#[derive(Debug, Clone, PartialEq)]
pub struct AttackDamageFlags {
    /// Armor flags.
    pub armor: ArmorDamageFlags,
    /// Skip knockback.
    pub no_knockback: bool,
    /// Skip protection.
    pub no_protection: bool,
    /// Skip team protection.
    pub no_team_protection: bool,
    /// Destroy armor.
    pub destroy_armor: bool,
}

/// Decode native damage flags by origin (`attackDamageFlags`).
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
        AttackCause::Q1 {
            armor_effect: Some(Q1ArmorEffect::Bypass),
            ..
        }
    );
    let half = matches!(
        cause,
        AttackCause::Q1 {
            armor_effect: Some(Q1ArmorEffect::HalfEffectiveness),
            ..
        }
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

/// Completed combat result (`CombatResult`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CombatResult {
    /// Applied damage.
    pub applied_damage: i32,
    /// Reaction.
    pub reaction: Reaction,
    /// Feedback.
    pub feedback: Option<DamageFeedback>,
}

/// Current combat state accessors (`CurrentCombatState`).
pub trait CurrentCombatState {
    /// Current target state.
    fn target(&self) -> Option<CombatState>;
    /// Current attacker state.
    fn attacker(&self) -> Option<CombatState>;
}

/// Combat progress (`CombatProgress`).
#[derive(Clone)]
#[allow(clippy::type_complexity, clippy::large_enum_variant)]
pub enum CombatProgress {
    /// Completed decision.
    Complete {
        /// Damage request.
        request: DamageRequest,
        /// Mutations.
        mutations: Vec<DamageMutation>,
        /// Result.
        result: CombatResult,
    },
    /// Armor stage awaiting its store.
    ArmorStage {
        /// Channel.
        channel: ProtectionChannel,
        /// Damage request.
        request: DamageRequest,
        /// Mutations so far.
        mutations: Vec<DamageMutation>,
        /// Stage input.
        input: ArmorStageInput,
        /// Fallback computation.
        fallback: Rc<dyn Fn(&ArmorState) -> ArmorResult>,
        /// Resume with a stage result.
        resume: Rc<dyn Fn(ArmorStageResult, &dyn CurrentCombatState) -> CombatProgress>,
    },
    /// Source continuation awaiting fresh state.
    SourceContinuation {
        /// Damage request.
        request: DamageRequest,
        /// Mutations so far.
        mutations: Vec<DamageMutation>,
        /// Resume with fresh state.
        resume: Rc<dyn Fn(&dyn CurrentCombatState) -> CombatProgress>,
    },
}

impl std::fmt::Debug for CombatProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Complete {
                request,
                mutations,
                result,
            } => f
                .debug_struct("Complete")
                .field("request", request)
                .field("mutations", mutations)
                .field("result", result)
                .finish(),
            Self::ArmorStage {
                channel,
                request,
                mutations,
                input,
                ..
            } => f
                .debug_struct("ArmorStage")
                .field("channel", channel)
                .field("request", request)
                .field("mutations", mutations)
                .field("input", input)
                .finish(),
            Self::SourceContinuation { request, mutations, .. } => f
                .debug_struct("SourceContinuation")
                .field("request", request)
                .field("mutations", mutations)
                .finish(),
        }
    }
}

/// Combat policy (`CombatPolicy`, decision layer).
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct CombatPolicy {
    /// Provider.
    pub id: ProviderId,
    /// Decide a request over target and attacker snapshots.
    pub decide: Rc<dyn Fn(&DamageRequest, &CombatState, Option<&CombatState>) -> CombatProgress>,
}

impl std::fmt::Debug for CombatPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CombatPolicy").field("id", &self.id).finish()
    }
}

/// Source damage modifier (`SourceDamageModifier`).
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct SourceDamageModifier {
    /// Owning provider.
    pub owner: ProviderId,
    /// Transform an attacker amount.
    pub transform: Rc<dyn Fn(Option<&ActorId>, f32) -> f32>,
}

impl std::fmt::Debug for SourceDamageModifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceDamageModifier")
            .field("owner", &self.owner)
            .finish()
    }
}

/// Quake III combat context (`Q3CombatContext`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3CombatContext {
    /// Target is a player.
    pub player: bool,
    /// Attacker is a player.
    pub attacker_player: bool,
    /// Attacker maximum health.
    pub attacker_max_health: i32,
    /// Attacker guard reduction applies.
    pub attacker_guard: bool,
    /// Intermission is queued.
    pub intermission: bool,
    /// Target noclips.
    pub noclip: bool,
    /// Missionpack invulnerability blocks.
    pub missionpack_invulnerability: bool,
    /// Target takes no knockback.
    pub no_knockback: bool,
    /// Knockback scale.
    pub knockback_scale: f32,
    /// Friendly fire.
    pub friendly_fire: bool,
    /// Battlesuit absorption.
    pub battlesuit: bool,
    /// Falling damage.
    pub falling: bool,
    /// Juiced damage.
    pub juiced: bool,
    /// Proximity protection.
    pub proximity_protected: bool,
    /// Product.
    pub product: Product,
}

pub(crate) fn combat_self_damage(request: &DamageRequest) -> bool {
    request
        .attack
        .attacker
        .as_ref()
        .is_some_and(|attacker| *attacker == request.target)
}

pub(crate) fn combat_same_team(target: &CombatState, attacker: Option<&CombatState>) -> bool {
    target
        .team
        .as_ref()
        .is_some_and(|team| !team.is_empty() && attacker.and_then(|state| state.team.as_ref()) == Some(team))
}

pub(crate) fn combat_decision(
    request: &DamageRequest,
    mutations: Vec<DamageMutation>,
    applied_damage: i32,
    reaction: Reaction,
    feedback: Option<DamageFeedback>,
) -> CombatProgress {
    CombatProgress::Complete {
        request: request.clone(),
        mutations,
        result: CombatResult {
            applied_damage,
            reaction,
            feedback,
        },
    }
}

#[allow(clippy::type_complexity)]
pub(crate) fn combat_continuation(
    request: &DamageRequest,
    mutations: Vec<DamageMutation>,
    resume: Rc<dyn Fn(&dyn CurrentCombatState) -> CombatProgress>,
) -> CombatProgress {
    CombatProgress::SourceContinuation {
        request: request.clone(),
        mutations,
        resume,
    }
}

#[allow(clippy::type_complexity)]
pub(crate) fn combat_armor_stage(
    channel: ProtectionChannel,
    request: &DamageRequest,
    amount: i32,
    flags: &ArmorDamageFlags,
    armor: &VictimArmorPolicy,
    resume: Rc<dyn Fn(i32, &dyn CurrentCombatState) -> CombatProgress>,
) -> CombatProgress {
    let stage = match channel {
        ProtectionChannel::Powered => ArmorStage::Power,
        ProtectionChannel::Regular => ArmorStage::Regular,
    };
    let mut staged = flags.clone();
    staged.stage = Some(stage);
    let input = ArmorStageInput {
        request: request.clone(),
        direction: request.direction,
        point: request.point,
        normal: request.normal,
        amount,
        flags: staged.clone(),
    };
    let fallback_armor = armor.clone();
    let fallback_request = request.clone();
    let fallback_amount = amount;
    CombatProgress::ArmorStage {
        channel,
        request: request.clone(),
        mutations: Vec::new(),
        input,
        fallback: Rc::new(move |current| fallback_armor(&fallback_request, current, fallback_amount, &staged)),
        resume: Rc::new(move |result, current| resume(result.saved, current)),
    }
}

pub(crate) fn combat_add_impulse(
    request: &DamageRequest,
    mutations: &mut Vec<DamageMutation>,
    direction: Vec3,
    amount: f32,
) {
    if amount != 0.0 {
        mutations.push(DamageMutation::Impulse {
            impulse: combat_scale(direction, amount),
            movement_provider: request.attack.movement_provider.clone(),
        });
    }
}

pub(crate) fn combat_scale(direction: Vec3, amount: f32) -> Vec3 {
    let x = direction.x;
    let y = direction.y;
    let z = direction.z;
    let length = (x * x + y * y + z * z).sqrt();
    if length == 0.0 {
        return vec3(0.0, 0.0, 0.0);
    }
    let inverse = 1.0 / length;
    vec3((x * inverse) * amount, (y * inverse) * amount, (z * inverse) * amount)
}

/// Quake III combat policy (`createQ3CombatPolicy`).
///
/// Request amounts truncate to `i32` damage words, matching the source's
/// integer pipeline for in-range amounts.
#[must_use]
#[allow(clippy::type_complexity)]
pub fn create_q3_combat_policy(
    id: ProviderId,
    armor: VictimArmorPolicy,
    context: Rc<dyn Fn(&DamageRequest, &CombatState, Option<&CombatState>) -> Q3CombatContext>,
) -> CombatPolicy {
    CombatPolicy {
        id,
        decide: Rc::new(move |request, target, attacker| {
            if !target.can_take_damage {
                return combat_decision(request, Vec::new(), 0, Reaction::None, None);
            }
            let context = context(request, target, attacker);
            if context.intermission || context.noclip || (context.missionpack_invulnerability && !context.juiced) {
                return combat_decision(request, Vec::new(), 0, Reaction::None, None);
            }
            let flags = attack_damage_flags(request);
            let mut damage = request.amount as i32;
            if context.attacker_player && !combat_self_damage(request) {
                let maximum = if context.attacker_guard {
                    context.attacker_max_health / 2
                } else {
                    context.attacker_max_health
                };
                damage = damage.wrapping_mul(maximum) / 100;
            }
            let mut mutations = Vec::new();
            let knockback = if context.no_knockback || target.no_knockback || flags.no_knockback {
                0
            } else {
                damage.min(200)
            };
            let battlesuit = Rc::new(RefCell::new(false));
            let finishing = |mutations: Vec<DamageMutation>,
                             applied: i32,
                             reaction: Reaction,
                             battlesuit: bool|
             -> CombatProgress {
                combat_decision(
                    request,
                    mutations,
                    applied,
                    reaction,
                    Some(DamageFeedback::Q3 { knockback, battlesuit }),
                )
            };
            if context.player && !context.no_knockback && !target.no_knockback && !flags.no_knockback {
                combat_add_impulse(
                    request,
                    &mut mutations,
                    request.direction,
                    (context.knockback_scale * (knockback as f32)) / 200.0,
                );
            }
            if !flags.no_protection {
                let check_team = context.product == Product::Baseq3 || (!context.juiced && !flags.no_team_protection);
                if (check_team
                    && !combat_self_damage(request)
                    && combat_same_team(target, attacker)
                    && !context.friendly_fire)
                    || context.proximity_protected
                    || target.invulnerable
                {
                    return finishing(mutations, 0, Reaction::None, false);
                }
            }
            if context.battlesuit {
                *battlesuit.borrow_mut() = true;
                if request.delivery == Delivery::Radius || context.falling {
                    return finishing(mutations, 0, Reaction::None, true);
                }
                damage /= 2;
            }
            if combat_self_damage(request) {
                damage /= 2;
            }
            damage = damage.max(1);
            let amount = damage;
            let armor_power = armor.clone();
            let armor_regular = armor.clone();
            let flags_power = flags.clone();
            let battlesuit_inner = battlesuit.clone();
            let owned = (*request).clone();
            combat_continuation(
                request,
                mutations,
                Rc::new({
                    let owned = owned.clone();
                    move |_| {
                        let flags_regular = flags_power.clone();
                        let armor_inner = armor_regular.clone();
                        let battlesuit = battlesuit_inner.clone();
                        let owned = owned.clone();
                        combat_armor_stage(
                            ProtectionChannel::Powered,
                            &owned,
                            amount,
                            &flags_power.armor,
                            &armor_power,
                            Rc::new({
                                let owned = owned.clone();
                                move |power_saved, current| {
                                    if current.target().is_none() {
                                        return combat_decision(&owned, Vec::new(), 0, Reaction::None, None);
                                    }
                                    let owned = owned.clone();
                                    let battlesuit = battlesuit.clone();
                                    combat_armor_stage(
                                        ProtectionChannel::Regular,
                                        &owned,
                                        amount.wrapping_sub(power_saved),
                                        &flags_regular.armor,
                                        &armor_inner,
                                        Rc::new({
                                            let owned = owned.clone();
                                            let battlesuit = battlesuit.clone();
                                            move |saved, state| {
                                                let Some(latest) = state.target() else {
                                                    return combat_decision(
                                                        &owned,
                                                        Vec::new(),
                                                        0,
                                                        Reaction::None,
                                                        None,
                                                    );
                                                };
                                                let take = amount.wrapping_sub(power_saved.wrapping_add(saved));
                                                let feedback = DamageFeedback::Q3 {
                                                    knockback,
                                                    battlesuit: *battlesuit.borrow(),
                                                };
                                                if take == 0 {
                                                    return combat_decision(
                                                        &owned,
                                                        Vec::new(),
                                                        0,
                                                        Reaction::None,
                                                        Some(feedback),
                                                    );
                                                }
                                                let health = (latest.health.wrapping_sub(take)).max(-999);
                                                combat_decision(
                                                    &owned,
                                                    vec![DamageMutation::Health {
                                                        before: latest.health,
                                                        after: health,
                                                    }],
                                                    take,
                                                    if health <= 0 { Reaction::Death } else { Reaction::Pain },
                                                    Some(feedback),
                                                )
                                            }
                                        }),
                                    )
                                }
                            }),
                        )
                    }
                }),
            )
        }),
    }
}

// ---------------------------------------------------------------------------
// game/combat.ts combat context (unified from base/mirrors.rs)
// ---------------------------------------------------------------------------

/// Combat actor services (`CombatContext` actors word, game/combat.ts).
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct CombatActors {
    /// Whether an actor is live.
    pub is_live: Rc<dyn Fn(&ActorId) -> bool>,
    /// Participant for an actor.
    pub participant: Rc<dyn Fn(&ActorId) -> DamageParticipant>,
    /// Projectile parent.
    pub parent: Rc<dyn Fn(&ActorId) -> Option<ActorId>>,
    /// Linked bounds.
    pub linked_bounds: Rc<dyn Fn(&ActorId) -> Option<Bounds>>,
    /// Whether an actor is a player.
    pub is_player: Rc<dyn Fn(&ActorId) -> bool>,
}

/// Combat context (`CombatContext`, game/combat.ts).
///
/// Product-specific words are `None` on baseq3, matching the donor's
/// discriminated union.
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct CombatContext {
    /// Product.
    pub product: Product,
    /// Gameplay authority.
    pub authority: Rc<dyn Q3SessionCombat>,
    /// Entity pool.
    pub entities: EntityPoolRef,
    /// Spatial queries.
    pub spatial: Rc<dyn Q3ServerWorld>,
    /// Source damage modifier.
    pub source_damage_modifier: Option<SourceDamageModifier>,
    /// Actor services.
    pub actors: CombatActors,
    /// Current time.
    pub time: Rc<dyn Fn() -> i32>,
    /// Queued intermission.
    pub intermission_queued: Rc<dyn Fn() -> i32>,
    /// Game type tag.
    pub game_type: Rc<dyn Fn() -> i32>,
    /// Friendly fire.
    pub friendly_fire: Rc<dyn Fn() -> bool>,
    /// Knockback scale.
    pub knockback: Rc<dyn Fn() -> f32>,
    /// Damage debug sink.
    pub debug_damage: Option<Rc<dyn Fn(DamageDiagnostic)>>,
    /// Capture attack provenance.
    pub attack: Rc<
        dyn Fn(&DamageParticipant, &DamageParticipant, Option<ItemId>, i32, i32, Option<ActorId>) -> AttackProvenance,
    >,
    /// Run an apply while retaining a source call.
    pub dispatch: Rc<dyn Fn(Q3DamageCall, &dyn Fn() -> DamageOutcome) -> DamageOutcome>,
    /// Carrier hurt hook.
    pub check_hurt_carrier: Rc<dyn Fn(EntityRef, EntityRef)>,
    /// Accuracy hit hook.
    pub log_accuracy_hit: Rc<dyn Fn(EntityRef, EntityRef) -> bool>,
    /// Obelisk attack hook (missionpack only).
    pub check_obelisk_attack: Option<Rc<dyn Fn(EntityRef, &DamageParticipant) -> bool>>,
    /// Invulnerability effect hook (missionpack only).
    pub invulnerability_effect: Option<Rc<dyn Fn(EntityRef, Vec3, Vec3)>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q3::base::game::state::MAX_CLIENTS;
    use crate::q3::base::records::test_support::*;
    use crate::q3::base::world_adapter::Q3WorldAdapter;

    struct FakePool {
        records: Q3EntityRecords,
        num: usize,
    }

    impl Q3EntityPool for FakePool {
        fn num_entities(&self) -> usize {
            self.num
        }

        fn entity_at(&self, index: usize) -> EntityRef {
            self.records.get(index).expect("pool entity")
        }
    }

    struct FakeBridgeHost {
        records_host: Rc<FakeRecordHost>,
        records: Q3EntityRecords,
        pool: EntityPoolRef,
        world: Rc<dyn Q3ServerWorld>,
        time: Cell<i32>,
        intermission: Cell<i32>,
        game_type: Cell<i32>,
        feedback: RefCell<Vec<String>>,
    }

    impl FakeBridgeHost {
        fn new(records_host: Rc<FakeRecordHost>, records: Q3EntityRecords) -> Rc<Self> {
            let world_host = Rc::new(FakeWorldHost::new());
            let world: Rc<dyn Q3ServerWorld> = Rc::new(Q3WorldAdapter::new(world_host, records.clone()));
            Rc::new(Self {
                records_host,
                pool: Rc::new(FakePool {
                    records: records.clone(),
                    num: MAX_CLIENTS,
                }),
                world,
                records,
                time: Cell::new(1000),
                intermission: Cell::new(0),
                game_type: Cell::new(0),
                feedback: RefCell::new(Vec::new()),
            })
        }
    }

    impl Q3CombatBridgeHost for FakeBridgeHost {
        fn authority(&self) -> Rc<dyn Q3SessionCombat> {
            self.records_host.combat.clone()
        }

        fn entities(&self) -> EntityPoolRef {
            self.pool.clone()
        }

        fn records(&self) -> Q3EntityRecords {
            self.records.clone()
        }

        fn world(&self) -> Rc<dyn Q3ServerWorld> {
            self.world.clone()
        }

        fn weapon_provider(&self) -> ProviderId {
            ProviderId::new("q3", "weapon")
        }

        fn combat_provider(&self) -> ProviderId {
            ProviderId::new("q3", "combat")
        }

        fn inventory_provider(&self) -> ProviderId {
            ProviderId::new("q3", "inventory")
        }

        fn movement_provider(&self) -> ProviderId {
            ProviderId::new("q3", "movement")
        }

        fn armor_context(&self, _request: &DamageRequest) -> VictimArmorContext {
            VictimArmorContext {
                screen_facing_dot: 0.0,
                arithmetic: VictimArithmetic::Binary32,
                q2: None,
            }
        }

        fn time(&self) -> i32 {
            self.time.get()
        }

        fn intermission_queued(&self) -> i32 {
            self.intermission.get()
        }

        fn game_type(&self) -> i32 {
            self.game_type.get()
        }

        fn friendly_fire(&self) -> bool {
            false
        }

        fn knockback(&self) -> f32 {
            1000.0
        }

        fn product(&self) -> Product {
            Product::Baseq3
        }

        fn check_hurt_carrier(&self, _target: EntityRef, _attacker: EntityRef) {}

        fn log_accuracy_hit(&self, _target: EntityRef, _attacker: EntityRef) -> bool {
            false
        }

        fn damage_feedback(&self, _call: &Q3DamageCall, _decision: &DamageDecision) {
            self.feedback.borrow_mut().push("damage".to_string());
        }

        fn foreign_damage_feedback(&self, _target: EntityRef, _owner: Option<EntityRef>, _decision: &DamageDecision) {
            self.feedback.borrow_mut().push("foreign".to_string());
        }
    }

    fn test_attack(target: &ActorId, attacker: Option<&ActorId>) -> AttackProvenance {
        let _ = target;
        AttackProvenance {
            sequence: 0,
            time: SourceTime::Milliseconds(1000),
            attacker: attacker.cloned(),
            inflictor: attacker.cloned(),
            originating_projectile: None,
            weapon: None,
            weapon_provider: ProviderId::new("q3", "weapon"),
            damage_powerup_owner: None,
            combat_provider: ProviderId::new("q3", "combat"),
            inventory_provider: ProviderId::new("q3", "inventory"),
            movement_provider: ProviderId::new("q3", "movement"),
            cause: AttackCause::Q3 {
                means_of_death: 7,
                damage_flags: 0,
            },
        }
    }

    fn test_request(target: ActorId, attacker: Option<ActorId>, amount: f32) -> DamageRequest {
        DamageRequest {
            attack: test_attack(&target, attacker.as_ref()),
            target,
            amount,
            knockback: amount,
            direction: vec3(1.0, 0.0, 0.0),
            point: vec3(0.0, 0.0, 0.0),
            normal: vec3(0.0, 0.0, 0.0),
            delivery: Delivery::Direct,
        }
    }

    fn test_combat_state(health: i32) -> CombatState {
        CombatState {
            health,
            armor: ArmorState {
                regular: RegularArmorState::Q3 {
                    points: 0,
                    protection: 0.66,
                },
                powered: PoweredProtectionState::None,
            },
            mass: 200,
            can_take_damage: true,
            invulnerable: false,
            no_knockback: false,
            team: None,
        }
    }

    struct FixedCurrent {
        target: Option<CombatState>,
        attacker: Option<CombatState>,
    }

    impl CurrentCombatState for FixedCurrent {
        fn target(&self) -> Option<CombatState> {
            self.target.clone()
        }

        fn attacker(&self) -> Option<CombatState> {
            self.attacker.clone()
        }
    }

    fn drive_progress(
        progress: CombatProgress,
        target: &CombatState,
        attacker: Option<&CombatState>,
    ) -> (CombatResult, Vec<DamageMutation>) {
        let mut progress = progress;
        let mut armor = target.armor.clone();
        let mut seen = Vec::new();
        loop {
            match progress {
                CombatProgress::Complete {
                    result, mut mutations, ..
                } => {
                    seen.append(&mut mutations);
                    return (result, seen);
                }
                CombatProgress::SourceContinuation {
                    mut mutations, resume, ..
                } => {
                    seen.append(&mut mutations);
                    let current = FixedCurrent {
                        target: Some(target.clone()),
                        attacker: attacker.cloned(),
                    };
                    progress = resume(&current);
                }
                CombatProgress::ArmorStage {
                    channel,
                    mut mutations,
                    resume,
                    fallback,
                    ..
                } => {
                    seen.append(&mut mutations);
                    let result = fallback(&armor);
                    armor = result.armor.clone();
                    let saved = match channel {
                        ProtectionChannel::Powered => result.power_saved,
                        ProtectionChannel::Regular => result.regular_saved,
                    };
                    let current = FixedCurrent {
                        target: Some(target.clone()),
                        attacker: attacker.cloned(),
                    };
                    progress = resume(ArmorStageResult { saved }, &current);
                }
            }
        }
    }

    #[test]
    fn bridge_captures_provenance_and_routes_feedback() {
        let (records_host, records) = test_records();
        let target = records.activate(5);
        target.borrow_mut().s.weapon = Weapon::WpShotgun as i32;
        let attacker = records.activate(6);
        let host = FakeBridgeHost::new(records_host, records.clone());
        let bridge = Q3CombatBridge::new(host.clone());

        let provenance = (bridge.context().attack)(
            &DamageParticipant::Native(target.clone()),
            &DamageParticipant::Native(attacker.clone()),
            None,
            7,
            0,
            None,
        );
        assert_eq!(provenance.sequence, 0);
        assert_eq!(provenance.weapon, Some("q3:weapon/shotgun".to_string()));
        let second = (bridge.context().attack)(
            &DamageParticipant::Native(target.clone()),
            &DamageParticipant::Native(attacker.clone()),
            Some("q3:weapon/bfg".to_string()),
            7,
            0,
            None,
        );
        assert_eq!(second.sequence, 1);
        assert_eq!(second.weapon, Some("q3:weapon/bfg".to_string()));

        let saved = bridge.capture_save_state();
        let bridge2 = Q3CombatBridge::new(host.clone());
        bridge2.restore_save_state(&saved).expect("restore");
        let third = (bridge2.context().attack)(
            &DamageParticipant::Native(target.clone()),
            &DamageParticipant::Native(attacker.clone()),
            None,
            7,
            0,
            None,
        );
        assert_eq!(third.sequence, 2);

        let target_id = target.borrow().actor().id().clone();
        let attacker_id = attacker.borrow().actor().id().clone();
        let decision = DamageDecision {
            request: test_request(target_id.clone(), Some(attacker_id), 40.0),
            mutations: Vec::new(),
            applied_damage: 40,
            reaction: Reaction::Pain,
            feedback: None,
        };
        let outcome = (bridge.context().dispatch)(
            Q3DamageCall {
                target: target.clone(),
                source: DamageParticipant::Native(attacker.clone()),
                owner: DamageParticipant::Native(attacker.clone()),
                direction: None,
                point: None,
                amount: 40.0,
                flags: 0,
                method_of_death: 7,
            },
            &|| DamageOutcome::StaleTarget {
                request: test_request(target_id.clone(), None, 0.0),
            },
        );
        assert!(matches!(outcome, DamageOutcome::StaleTarget { .. }));
        assert!(bridge.current_call().is_none());
        bridge.before_reaction(&DamageDecision {
            request: test_request(target_id, None, 10.0),
            ..decision.clone()
        });
        assert_eq!(*host.feedback.borrow(), vec!["foreign".to_string()]);
    }

    #[test]
    fn bridge_policy_decides_q3_damage_flow() {
        let (records_host, records) = test_records();
        let target = records.activate(5);
        target.borrow_mut().client = Some(records.client(5));
        let attacker = records.activate(6);
        attacker.borrow_mut().client = Some(records.client(6));
        attacker
            .borrow()
            .client
            .as_ref()
            .unwrap()
            .borrow_mut()
            .ps
            .stats
            .set(BaseStatIndex::StatMaxHealth as usize, 100);
        let host = FakeBridgeHost::new(records_host, records);
        let bridge = Q3CombatBridge::new(host.clone());
        let policy = bridge.policy();

        let target_id = target.borrow().actor().id().clone();
        let attacker_id = attacker.borrow().actor().id().clone();
        let request = test_request(target_id, Some(attacker_id), 50.0);
        let state = test_combat_state(100);
        let progress = (policy.decide)(&request, &state, Some(&state));
        let (result, mutations) = drive_progress(progress, &state, Some(&state));
        assert_eq!(result.applied_damage, 50);
        assert_eq!(result.reaction, Reaction::Pain);
        assert!(mutations
            .iter()
            .any(|mutation| matches!(mutation, DamageMutation::Health { before: 100, after: 50 })));
        assert!(mutations
            .iter()
            .any(|mutation| matches!(mutation, DamageMutation::Impulse { .. })));

        host.intermission.set(1);
        let held = (policy.decide)(&request, &state, Some(&state));
        let (held_result, _) = drive_progress(held, &state, Some(&state));
        assert_eq!(held_result.applied_damage, 0);
        assert_eq!(held_result.reaction, Reaction::None);
    }

    #[test]
    fn bridge_policy_blocks_godmode_targets() {
        let (records_host, records) = test_records();
        let target = records.activate(5);
        target.borrow_mut().flags |= GameFlags::GODMODE;
        let host = FakeBridgeHost::new(records_host, records);
        let bridge = Q3CombatBridge::new(host);
        let policy = bridge.policy();
        let target_id = target.borrow().actor().id().clone();
        let request = test_request(target_id, None, 50.0);
        let state = test_combat_state(100);
        let (result, _) = drive_progress((policy.decide)(&request, &state, None), &state, None);
        assert_eq!(result.applied_damage, 0);
    }

    #[test]
    fn native_armor_absorbs_q3_points() {
        let armor = ArmorState {
            regular: RegularArmorState::Q3 {
                points: 50,
                protection: 0.66,
            },
            powered: PoweredProtectionState::None,
        };
        let flags = ArmorDamageFlags {
            stage: None,
            no_armor: false,
            no_power_armor: false,
            no_regular_armor: false,
            energy: false,
            regular_protection_scale: Some(1.0),
        };
        let context = VictimArmorContext {
            screen_facing_dot: 0.0,
            arithmetic: VictimArithmetic::Binary32,
            q2: None,
        };
        let result = absorb_native_armor(&armor, 100, &flags, &context);
        assert_eq!(result.regular_saved, 50);
        assert_eq!(result.power_saved, 0);
    }

    #[test]
    fn damage_flags_follow_shared_decoder() {
        use qa_core::identity::IdentityOwner;

        let owner = IdentityOwner::create("test").expect("owner");
        let target = owner.actor(1, 0);
        let mut q3 = test_request(target.clone(), None, 10.0);
        q3.attack.cause = AttackCause::Q3 {
            means_of_death: 7,
            damage_flags: 4 | 8 | 0x10,
        };
        let flags = attack_damage_flags(&q3);
        assert!(!flags.armor.no_armor);
        assert!(flags.no_knockback);
        assert!(flags.no_protection);
        assert!(flags.no_team_protection);
        assert!(!flags.destroy_armor);

        let mut bypass = test_request(target, None, 10.0);
        bypass.attack.cause = AttackCause::Q1 {
            death_type: String::from("test"),
            armor_effect: Some(Q1ArmorEffect::Bypass),
        };
        let flags = attack_damage_flags(&bypass);
        assert!(flags.armor.no_armor);
        assert_eq!(flags.armor.regular_protection_scale, Some(1.0));
    }
}
