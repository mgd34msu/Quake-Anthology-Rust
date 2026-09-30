//! Quake III base: combat bridge.
//!
//! Donor provenance: `src/content/q3/base/combat-bridge.ts`.

use crate::value::{int, obj, SaveJson, SaveReader, ValueError};
use qa_core::identity::{ActorId, ProviderId};
use qa_core::math::{Bounds, Vec3};
use qa_core::time::SourceTime;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::mirrors::*;
use crate::q3::base::records::*;
use crate::q3::base::shared::definitions::*;
use crate::q3::base::shared::items::*;
use crate::q3::base::world::*;

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
                .is_some_and(|entity| entity.borrow().flags & GameFlags::NoKnockback.bits() != 0),
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
                .is_some_and(|entity| entity.borrow().flags & GameFlags::Godmode.bits() != 0);
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
