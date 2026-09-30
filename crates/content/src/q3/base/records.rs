//! Quake III base: records.
//!
//! Donor provenance: `src/content/q3/base/records.ts`.

use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::math::{vec3, Bounds, Vec3};
use qa_core::time::SourceTime;
use qa_world::body::{BodyState, LinkedBody};
use qa_world::combat::{Delivery, Reaction};
use thiserror::Error;

use crate::contract::ItemId;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::state::{MAX_CLIENTS, MAX_GENTITIES};
use crate::q3::base::shared::definitions::*;
use crate::q3::base::shared::entity_shared::*;
use crate::q3::base::shared::entity_state::*;
use crate::q3::base::shared::player_state::*;
use crate::q3::foundation::arsenal::{q3_weapon_item, Q3_WEAPON_ITEMS};

// ---------------------------------------------------------------------------
// records.ts
// ---------------------------------------------------------------------------

/// Record host services (`Q3RecordHost`).
pub trait Q3RecordHost {
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
    /// Ammunition timer store notification.
    fn ammo_timer_stored(&self, _actor: &ActorId, _weapon: usize, _value: i32) {}
    /// Schedule an actor think.
    fn schedule(&self, actor: &OwnedActor, due_milliseconds: Option<i32>);
    /// Run an actor think.
    fn run_think(&self, actor: &OwnedActor, time_milliseconds: i32);
    /// Innermost retained source damage call.
    fn damage_call(&self) -> Option<Q3DamageCall>;
    /// Admit damage for an entity.
    fn admit_damage(&self, _entity: EntityRef, _request: &DamageRequest) -> DamageAdmission {
        DamageAdmission::Continue
    }
    /// Project a foreign actor into the source-slot view.
    fn foreign(&self, actor: &ActorId) -> Option<EntityRef>;
    /// Whether an actor is a player.
    fn is_player(&self, actor: &ActorId) -> bool;
}

/// Owned slot state for save capture (`captureOwnership` words).
#[derive(Debug, Clone, PartialEq)]
pub struct SlotOwnership {
    /// Owned actor.
    pub actor: Option<OwnedActor>,
    /// Active.
    pub active: bool,
    /// Borrowed.
    pub borrowed: bool,
}

/// Private client backing snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientBackingSnapshot {
    /// Source stats.
    pub source_stats: [i32; 16],
    /// Special ammunition.
    pub special_ammo: [i32; 16],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClientBacking {
    source_stats: [i32; 16],
    special_ammo: [i32; 16],
}

pub(crate) struct SourceRecord {
    entity: EntityRef,
    actor: Option<OwnedActor>,
    active: bool,
    borrowed: bool,
}

pub(crate) struct RecordsInner {
    slots: Vec<SourceRecord>,
    clients: Vec<ClientRef>,
    backing: Vec<ClientBacking>,
    unobserve: Option<Box<dyn Fn()>>,
}

pub(crate) struct RecordsCore {
    host: Rc<dyn Q3RecordHost>,
    provider: ProviderId,
    product: Product,
    inner: RefCell<RecordsInner>,
}

/// `gentity_t` private records (`Q3EntityRecords`).
///
/// Lifetime, body, combat, and inventory come from the session owners;
/// the records own source slots, clients, and private backing.
#[derive(Clone)]
pub struct Q3EntityRecords {
    core: Rc<RecordsCore>,
}

impl std::fmt::Debug for Q3EntityRecords {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Q3EntityRecords")
            .field("provider", &self.core.provider)
            .field("product", &self.core.product)
            .finish()
    }
}

pub(crate) struct RecordBodyBinding {
    core: Weak<RecordsCore>,
    slot: usize,
}

impl EntityBodyBinding for RecordBodyBinding {
    fn read(&self) -> BodyState {
        let Some(core) = self.core.upgrade() else {
            return ZERO_BODY.clone();
        };
        let actor = core.record_actor(self.slot);
        actor.as_ref().map_or_else(
            || ZERO_BODY.clone(),
            |owned| core.host.bodies().read(owned.id()).unwrap_or_else(|| ZERO_BODY.clone()),
        )
    }

    fn write(&self, value: BodyState) {
        if let Some(core) = self.core.upgrade() {
            let actor = core.ensure_actor(self.slot);
            core.host.bodies().write(&actor, value);
        }
    }

    fn linked(&self) -> Option<LinkedBody> {
        let core = self.core.upgrade()?;
        let actor = core.record_actor(self.slot)?;
        core.host.bodies().linked(actor.id())
    }
}

pub(crate) struct RecordEntityBinding {
    core: Weak<RecordsCore>,
    slot: usize,
    body: Rc<dyn EntityBodyBinding>,
}

impl GameEntityBinding for RecordEntityBinding {
    fn body(&self) -> Rc<dyn EntityBodyBinding> {
        self.body.clone()
    }

    fn actor(&self) -> OwnedActor {
        self.core
            .upgrade()
            .unwrap_or_else(|| panic!("Q3 records are closed"))
            .ensure_actor(self.slot)
    }

    fn active(&self) -> bool {
        self.core.upgrade().is_some_and(|core| {
            if !core.record_active(self.slot) {
                return false;
            }
            let actor = core.ensure_actor(self.slot);
            core.host.actors().is_live(actor.id())
        })
    }

    fn health(&self) -> i32 {
        self.core.upgrade().map_or(0, |core| {
            core.record_actor(self.slot).map_or(0, |actor| {
                core.host.combat().read(actor.id()).map_or(0, |state| state.health)
            })
        })
    }

    fn set_health(&self, value: i32) {
        if let Some(core) = self.core.upgrade() {
            let actor = core.ensure_actor(self.slot);
            core.host.combat().set_health(&actor, value);
        }
    }

    fn takes_damage(&self) -> bool {
        self.core.upgrade().is_some_and(|core| {
            core.record_actor(self.slot).is_some_and(|actor| {
                core.host
                    .combat()
                    .read(actor.id())
                    .is_some_and(|state| state.can_take_damage)
            })
        })
    }

    fn set_takes_damage(&self, value: bool) {
        if let Some(core) = self.core.upgrade() {
            let actor = core.ensure_actor(self.slot);
            core.host.combat().set_can_take_damage(&actor, value);
        }
    }

    fn schedule(&self, nextthink: i32) {
        if let Some(core) = self.core.upgrade() {
            if let Some(actor) = core.record_actor(self.slot) {
                core.host
                    .schedule(&actor, if nextthink <= 0 { None } else { Some(nextthink) });
            }
        }
    }

    fn run_think(&self, time_milliseconds: i32) {
        if let Some(core) = self.core.upgrade() {
            let actor = core.ensure_actor(self.slot);
            core.host.run_think(&actor, time_milliseconds);
        }
    }
}

pub(crate) struct RecordStatBinding {
    core: Weak<RecordsCore>,
    slot: usize,
}

impl PlayerSlotBinding for RecordStatBinding {
    fn read(&self, index: usize) -> i32 {
        self.core
            .upgrade()
            .unwrap_or_else(|| panic!("Q3 records are closed"))
            .stat_read(self.slot, index)
    }

    fn write(&self, index: usize, value: i32) {
        if let Some(core) = self.core.upgrade() {
            core.stat_write(self.slot, index, value);
        }
    }
}

pub(crate) struct RecordAmmoBinding {
    core: Weak<RecordsCore>,
    slot: usize,
}

impl PlayerSlotBinding for RecordAmmoBinding {
    fn read(&self, index: usize) -> i32 {
        self.core
            .upgrade()
            .unwrap_or_else(|| panic!("Q3 records are closed"))
            .ammo_read(self.slot, index)
    }

    fn write(&self, index: usize, value: i32) {
        if let Some(core) = self.core.upgrade() {
            core.ammo_write(self.slot, index, value);
        }
    }
}

pub(crate) struct RecordPlayerAuthority {
    core: Weak<RecordsCore>,
    slot: usize,
    stats: Rc<dyn PlayerSlotBinding>,
    ammo: Rc<dyn PlayerSlotBinding>,
}

impl PlayerAuthorityBinding for RecordPlayerAuthority {
    fn origin(&self) -> Vec3 {
        self.core
            .upgrade()
            .map_or_else(|| vec3(0.0, 0.0, 0.0), |core| core.record_body(self.slot).origin)
    }

    fn set_origin(&self, value: Vec3) {
        if let Some(core) = self.core.upgrade() {
            let mut body = core.record_body(self.slot);
            body.origin = value;
            let actor = core.ensure_actor(self.slot);
            core.host.bodies().write(&actor, body);
        }
    }

    fn read_velocity(&self) -> Vec3 {
        self.core
            .upgrade()
            .map_or_else(|| vec3(0.0, 0.0, 0.0), |core| core.record_body(self.slot).velocity)
    }

    fn set_velocity(&self, value: Vec3) {
        if let Some(core) = self.core.upgrade() {
            let mut body = core.record_body(self.slot);
            body.velocity = value;
            let actor = core.ensure_actor(self.slot);
            core.host.bodies().write(&actor, body);
        }
    }

    fn stats(&self) -> Rc<dyn PlayerSlotBinding> {
        self.stats.clone()
    }

    fn ammo(&self) -> Rc<dyn PlayerSlotBinding> {
        self.ammo.clone()
    }
}

impl RecordsCore {
    fn record_entity(self: &Rc<Self>, slot: usize) -> EntityRef {
        self.inner
            .borrow()
            .slots
            .get(slot)
            .unwrap_or_else(|| panic!("Q3 entity {slot} outside 0..1023"))
            .entity
            .clone()
    }

    fn record_actor(self: &Rc<Self>, slot: usize) -> Option<OwnedActor> {
        self.inner
            .borrow()
            .slots
            .get(slot)
            .unwrap_or_else(|| panic!("Q3 entity {slot} outside 0..1023"))
            .actor
            .clone()
    }

    fn record_active(self: &Rc<Self>, slot: usize) -> bool {
        self.inner
            .borrow()
            .slots
            .get(slot)
            .unwrap_or_else(|| panic!("Q3 entity {slot} outside 0..1023"))
            .active
    }

    fn client_ref(self: &Rc<Self>, slot: usize) -> ClientRef {
        self.inner
            .borrow()
            .clients
            .get(slot)
            .unwrap_or_else(|| panic!("Q3 client {slot} outside 0..63"))
            .clone()
    }

    fn record_body(self: &Rc<Self>, slot: usize) -> BodyState {
        let actor = self.record_actor(slot);
        actor.map_or_else(
            || ZERO_BODY.clone(),
            |owned| self.host.bodies().read(owned.id()).unwrap_or_else(|| ZERO_BODY.clone()),
        )
    }

    fn ensure_actor(self: &Rc<Self>, slot: usize) -> OwnedActor {
        if let Some(actor) = self.record_actor(slot) {
            self.host.actors().assert_owned(&actor).expect("Q3 actor ownership");
            return actor;
        }
        let definition = if slot < MAX_CLIENTS { "q3:player" } else { "q3:entity" };
        let actor = self.host.actors().allocate_at_source(&self.provider, slot, definition);
        self.host.bodies().create(&actor, ZERO_BODY.clone());
        self.inner.borrow_mut().slots[slot].actor = Some(actor.clone());
        self.bind_owned_services(slot);
        actor
    }

    fn admit_fn(self: &Rc<Self>, slot: usize) -> DamageAdmissionFn {
        let host = self.host.clone();
        let entity = self.record_entity(slot);
        Rc::new(move |request| host.admit_damage(entity.clone(), request))
    }

    fn bind_owned_services(self: &Rc<Self>, slot: usize) {
        let Some(actor) = self.record_actor(slot) else {
            panic!("Cannot bind Q3 services without an actor");
        };
        if self.host.combat().read(actor.id()).is_none() {
            self.host.combat().create(
                &actor,
                CombatState {
                    health: 0,
                    armor: ArmorState {
                        regular: RegularArmorState::Q3 {
                            points: 0,
                            protection: 0.66,
                        },
                        powered: PoweredProtectionState::None,
                    },
                    mass: 200,
                    can_take_damage: false,
                    invulnerable: false,
                    no_knockback: false,
                    team: None,
                },
                Some(self.admit_fn(slot)),
            );
        } else {
            self.host.combat().bind_damage_admission(&actor, self.admit_fn(slot));
        }
        if !self.host.inventory().has(actor.id()) {
            self.host.inventory().create(&actor, Vec::new());
        }
        self.bind_callbacks(slot);
    }

    fn native_by_actor(self: &Rc<Self>, actor: Option<&ActorId>) -> Option<EntityRef> {
        let actor = actor?;
        self.inner
            .borrow()
            .slots
            .iter()
            .find(|record| record.actor.as_ref().is_some_and(|owned| owned.id() == actor))
            .map(|record| record.entity.clone())
    }

    fn by_actor(self: &Rc<Self>, actor: Option<&ActorId>) -> Option<EntityRef> {
        let actor = actor?;
        if let Some(native) = self.native_by_actor(Some(actor)) {
            return Some(native);
        }
        self.host.foreign(actor)
    }

    fn damage_inflictor(self: &Rc<Self>, actor: Option<&ActorId>) -> DamageParticipant {
        let Some(actor) = actor else {
            return DamageParticipant::Native(self.record_entity(ENTITYNUM_WORLD as usize));
        };
        if let Some(native) = self.native_by_actor(Some(actor)) {
            return DamageParticipant::Native(native);
        }
        let host = self.host.clone();
        let actor = actor.clone();
        DamageParticipant::SharedActor(SharedParticipant::new(
            actor.clone(),
            Rc::new(move || {
                let body = host.bodies().read(&actor)?;
                if host.actors().is_live(&actor) {
                    Some(body.origin)
                } else {
                    None
                }
            }),
        ))
    }

    fn use_participant(self: &Rc<Self>, actor: Option<&ActorId>) -> Option<DamageParticipant> {
        actor.map(|actor| self.damage_inflictor(Some(actor)))
    }

    fn bind_callbacks(self: &Rc<Self>, slot: usize) {
        let Some(actor) = self.record_actor(slot) else {
            panic!("Cannot bind inactive Q3 record callbacks");
        };
        let entity = self.record_entity(slot);
        let host = self.host.clone();
        let think_entity = entity.clone();
        let think = Rc::new(move || {
            let callback = {
                let mut borrowed = think_entity.borrow_mut();
                borrowed.set_nextthink(0);
                borrowed.think.clone()
            };
            let Some(callback) = callback else {
                panic!("NULL ent->think");
            };
            callback(think_entity.clone());
        });
        let touch_entity = entity.clone();
        let touch_core = Rc::downgrade(self);
        let touch = Rc::new(move |contact: &TouchContact| {
            let callback = touch_entity.borrow().touch.clone();
            let Some(callback) = callback else {
                return;
            };
            let other = touch_core.upgrade().map_or_else(
                || DamageParticipant::SharedActor(SharedParticipant::new(contact.other.clone(), Rc::new(|| None))),
                |core| core.damage_inflictor(Some(&contact.other)),
            );
            // Native entities resolve through the inflictor's native
            // branch, matching the donor's slot lookup.
            callback(touch_entity.clone(), other, contact.clone());
        });
        let use_entity = entity.clone();
        let use_core = Rc::downgrade(self);
        let use_action = Rc::new(move |other: Option<ActorId>, activator: Option<ActorId>| {
            let callback = use_entity.borrow().use_action.clone();
            if let Some(callback) = callback {
                let (other, activator) = use_core.upgrade().map_or((None, None), |core| {
                    (
                        core.use_participant(other.as_ref()),
                        core.use_participant(activator.as_ref()),
                    )
                });
                callback(use_entity.clone(), other, activator);
            }
        });
        let pain_entity = entity.clone();
        let pain_core = Rc::downgrade(self);
        let pain = Rc::new(move |reaction: &PainReaction| {
            let callback = pain_entity.borrow().pain.clone();
            if let Some(callback) = callback {
                let other = pain_core.upgrade().map_or_else(
                    || {
                        DamageParticipant::SharedActor(SharedParticipant::new(
                            reaction
                                .attacker
                                .clone()
                                .unwrap_or_else(|| use_actor(&DamageParticipant::Native(pain_entity.clone()))),
                            Rc::new(|| None),
                        ))
                    },
                    |core| core.damage_inflictor(reaction.attacker.as_ref()),
                );
                callback(pain_entity.clone(), other, reaction.damage);
            }
        });
        let die_entity = entity.clone();
        let die_core = Rc::downgrade(self);
        let die_host = host.clone();
        let die = Rc::new(move |reaction: &DeathReaction| {
            let call = die_host.damage_call();
            let callback = die_entity.borrow().die.clone();
            let Some(callback) = callback else {
                panic!("G_Damage lethal target has no die callback");
            };
            let method_of_death = call.map_or_else(
                || match &reaction.attack {
                    Some(attack) => match &attack.cause {
                        AttackCause::Q3 { means_of_death, .. } => *means_of_death,
                        _ => 0,
                    },
                    None => 0,
                },
                |call| call.method_of_death,
            );
            let (inflictor, attacker) = die_core.upgrade().map_or_else(
                || {
                    (
                        DamageParticipant::Native(die_entity.clone()),
                        DamageParticipant::Native(die_entity.clone()),
                    )
                },
                |core| {
                    (
                        core.damage_inflictor(reaction.inflictor.as_ref()),
                        core.damage_inflictor(reaction.attacker.as_ref()),
                    )
                },
            );
            callback(
                die_entity.clone(),
                inflictor,
                attacker,
                reaction.damage,
                method_of_death,
            );
        });
        host.callbacks().bind(
            &actor,
            ActorCallbacks {
                think,
                touch,
                use_action,
                pain,
                die,
            },
        );
    }

    fn stat_read(self: &Rc<Self>, slot: usize, index: usize) -> i32 {
        let (health, armor, weapons) = match stat_schema(self.product) {
            StatSchema::Base(layout) => (layout.health, layout.armor, layout.weapons),
            StatSchema::Missionpack(layout) => (layout.health, layout.armor, layout.weapons),
        };
        let (health, armor, weapons) = (health as usize, armor as usize, weapons as usize);
        if index == health {
            return self.record_entity(slot).borrow().health();
        }
        if index == armor {
            let actor = self.record_actor(slot);
            return actor.map_or(0, |owned| {
                match self.host.combat().read(owned.id()).map(|state| state.armor.regular) {
                    Some(RegularArmorState::Q3 { points, .. })
                    | Some(RegularArmorState::Q1 { points, .. })
                    | Some(RegularArmorState::Q2 { points, .. })
                    | Some(RegularArmorState::Source { points, .. }) => points,
                    Some(RegularArmorState::None) | None => 0,
                }
            });
        }
        if index == weapons {
            let Some(actor) = self.record_actor(slot) else {
                return 0;
            };
            return Q3_WEAPON_ITEMS.iter().fold(0, |bits, weapon| {
                bits | i32::from(self.host.inventory().count(actor.id(), &weapon.item) > 0) << (weapon.weapon as i32)
            });
        }
        *self
            .inner
            .borrow()
            .backing
            .get(slot)
            .unwrap_or_else(|| panic!("Q3 client backing slot outside 0..63"))
            .source_stats
            .get(index)
            .unwrap_or_else(|| panic!("Q3 stat {index} outside 0..15"))
    }

    fn stat_write(self: &Rc<Self>, slot: usize, index: usize, value: i32) {
        let (health, armor, weapons) = match stat_schema(self.product) {
            StatSchema::Base(layout) => (layout.health, layout.armor, layout.weapons),
            StatSchema::Missionpack(layout) => (layout.health, layout.armor, layout.weapons),
        };
        let (health, armor, weapons) = (health as usize, armor as usize, weapons as usize);
        if index == health {
            let actor = self.ensure_actor(slot);
            self.host.combat().set_health(&actor, value);
            return;
        }
        if index == armor {
            let actor = self.ensure_actor(slot);
            self.host.combat().set_regular_points(
                &actor,
                value,
                RegularArmorState::Q3 {
                    points: value,
                    protection: 0.66,
                },
            );
            return;
        }
        if index == weapons {
            let actor = self.ensure_actor(slot);
            for weapon in Q3_WEAPON_ITEMS.iter() {
                self.host.inventory().configure(
                    &actor,
                    &weapon.item,
                    i32::from(value & (1 << (weapon.weapon as i32)) != 0),
                    1,
                );
            }
            return;
        }
        if index >= 16 {
            panic!("Q3 stat {index} outside 0..15");
        }
        self.inner.borrow_mut().backing[slot].source_stats[index] = value;
    }

    fn ammo_read(self: &Rc<Self>, slot: usize, index: usize) -> i32 {
        if let Some(weapon) = index
            .try_into()
            .ok()
            .and_then(q3_weapon_item)
            .and_then(|weapon| weapon.ammo.clone())
        {
            return self
                .record_actor(slot)
                .map_or(0, |actor| self.host.inventory().count(actor.id(), &weapon));
        }
        *self
            .inner
            .borrow()
            .backing
            .get(slot)
            .unwrap_or_else(|| panic!("Q3 client backing slot outside 0..63"))
            .special_ammo
            .get(index)
            .unwrap_or_else(|| panic!("Q3 ammo {index} outside 0..15"))
    }

    fn ammo_write(self: &Rc<Self>, slot: usize, index: usize, value: i32) {
        if let Some(weapon) = index
            .try_into()
            .ok()
            .and_then(q3_weapon_item)
            .and_then(|weapon| weapon.ammo.clone())
        {
            let actor = self.ensure_actor(slot);
            self.host.inventory().configure(&actor, &weapon, value, 200);
            return;
        }
        if index >= 16 {
            panic!("Q3 ammo {index} outside 0..15");
        }
        self.inner.borrow_mut().backing[slot].special_ammo[index] = value;
    }
}

impl Q3EntityRecords {
    /// Records over a session host, provider, and product.
    #[must_use]
    pub fn new(host: Rc<dyn Q3RecordHost>, provider: ProviderId, product: Product) -> Self {
        let core = Rc::new_cyclic(|weak: &Weak<RecordsCore>| {
            let mut slots = Vec::with_capacity(MAX_GENTITIES);
            for slot in 0..MAX_GENTITIES {
                let body: Rc<dyn EntityBodyBinding> = Rc::new(RecordBodyBinding {
                    core: weak.clone(),
                    slot,
                });
                let binding: Rc<dyn GameEntityBinding> = Rc::new(RecordEntityBinding {
                    core: weak.clone(),
                    slot,
                    body,
                });
                slots.push(SourceRecord {
                    entity: Rc::new(RefCell::new(GameEntity::new(slot, binding))),
                    actor: None,
                    active: false,
                    borrowed: false,
                });
            }
            let backing = vec![
                ClientBacking {
                    source_stats: [0; 16],
                    special_ammo: [0; 16],
                };
                MAX_CLIENTS
            ];
            let mut clients = Vec::with_capacity(MAX_CLIENTS);
            for slot in 0..MAX_CLIENTS {
                let stats: Rc<dyn PlayerSlotBinding> = Rc::new(RecordStatBinding {
                    core: weak.clone(),
                    slot,
                });
                let ammo: Rc<dyn PlayerSlotBinding> = Rc::new(RecordAmmoBinding {
                    core: weak.clone(),
                    slot,
                });
                let authority: Rc<dyn PlayerAuthorityBinding> = Rc::new(RecordPlayerAuthority {
                    core: weak.clone(),
                    slot,
                    stats,
                    ammo,
                });
                let notify = weak.clone();
                let notify_host = host.clone();
                let stored: Rc<dyn Fn(usize, i32)> = Rc::new(move |weapon, value| {
                    if let Some(core) = notify.upgrade() {
                        if let Some(actor) = core.record_actor(slot) {
                            notify_host.ammo_timer_stored(actor.id(), weapon, value);
                        }
                    }
                });
                clients.push(Rc::new(RefCell::new(GameClient::new(
                    product,
                    Some(authority),
                    Some(stored),
                ))));
            }
            RecordsCore {
                host,
                provider,
                product,
                inner: RefCell::new(RecordsInner {
                    slots,
                    clients,
                    backing,
                    unobserve: None,
                }),
            }
        });
        let release = Rc::downgrade(&core);
        let unobserve = core.host.actors().on_release(Box::new(move |actor| {
            if let Some(core) = release.upgrade() {
                let mut inner = core.inner.borrow_mut();
                for record in &mut inner.slots {
                    if record.actor.as_ref() == Some(actor) {
                        record.actor = None;
                        record.active = false;
                    }
                }
            }
        }));
        core.inner.borrow_mut().unobserve = Some(unobserve);
        Self { core }
    }

    /// Session host.
    #[must_use]
    pub fn host(&self) -> Rc<dyn Q3RecordHost> {
        self.core.host.clone()
    }

    /// Owning provider.
    #[must_use]
    pub fn provider(&self) -> &ProviderId {
        &self.core.provider
    }

    /// Product.
    #[must_use]
    pub fn product(&self) -> Product {
        self.core.product
    }

    /// Stop observing actor release.
    pub fn close(&self) {
        if let Some(unobserve) = self.core.inner.borrow_mut().unobserve.take() {
            unobserve();
        }
    }

    /// Entity at a slot.
    #[must_use]
    pub fn get(&self, slot: usize) -> Option<EntityRef> {
        self.core
            .inner
            .borrow()
            .slots
            .get(slot)
            .map(|record| record.entity.clone())
    }

    /// Client at a slot.
    ///
    /// # Panics
    ///
    /// Panics when `slot` is outside `0..63`.
    #[must_use]
    pub fn client(&self, slot: usize) -> ClientRef {
        self.core.client_ref(slot)
    }

    /// Capture slot ownership words.
    #[must_use]
    pub fn capture_ownership(&self) -> Vec<SlotOwnership> {
        self.core
            .inner
            .borrow()
            .slots
            .iter()
            .map(|record| SlotOwnership {
                actor: record.actor.clone(),
                active: record.active,
                borrowed: record.borrowed,
            })
            .collect()
    }

    /// Restore slot ownership words into fresh records.
    pub fn restore_ownership(&self, states: &[SlotOwnership]) -> Result<(), Q3BaseError> {
        if states.len() != MAX_GENTITIES {
            return Err(Q3BaseError::Invalid(
                "Restored Q3 ownership requires all retained slots".to_string(),
            ));
        }
        let mut seen: Vec<&OwnedActor> = Vec::new();
        for state in states {
            if let Some(actor) = &state.actor {
                self.core.host.actors().assert_owned(actor)?;
                if seen.contains(&actor) {
                    return Err(Q3BaseError::Invalid("Duplicate restored Q3 actor".to_string()));
                }
                seen.push(actor);
                if !state.borrowed && actor.owner() != &self.core.provider {
                    return Err(Q3BaseError::Invalid(
                        "Q3 owned actor has a foreign provider".to_string(),
                    ));
                }
            } else if state.active {
                return Err(Q3BaseError::Invalid("Active Q3 slot has no actor".to_string()));
            }
        }
        let mut inner = self.core.inner.borrow_mut();
        for (slot, state) in states.iter().enumerate() {
            let record = &mut inner.slots[slot];
            if record.actor.is_some() {
                return Err(Q3BaseError::Invalid(
                    "Q3 ownership hydration requires fresh records".to_string(),
                ));
            }
            record.actor = state.actor.clone();
            record.active = state.active;
            record.borrowed = state.borrowed;
        }
        Ok(())
    }

    /// Rebind callbacks for restored owned records.
    pub fn restore_callbacks(&self) {
        let owned: Vec<usize> = self
            .core
            .inner
            .borrow()
            .slots
            .iter()
            .enumerate()
            .filter(|(_, record)| record.actor.is_some() && !record.borrowed)
            .map(|(slot, _)| slot)
            .collect();
        for slot in owned {
            if let Some(actor) = self.core.record_actor(slot) {
                self.core
                    .host
                    .combat()
                    .bind_damage_admission(&actor, self.core.admit_fn(slot));
            }
            self.core.bind_callbacks(slot);
        }
    }

    /// Capture private client backing.
    ///
    /// # Panics
    ///
    /// Panics when `slot` is outside `0..63`.
    #[must_use]
    pub fn capture_client_backing(&self, slot: usize) -> ClientBackingSnapshot {
        let inner = self.core.inner.borrow();
        let Some(backing) = inner.backing.get(slot) else {
            panic!("Q3 client backing slot outside 0..63");
        };
        ClientBackingSnapshot {
            source_stats: backing.source_stats,
            special_ammo: backing.special_ammo,
        }
    }

    /// Restore private client backing.
    pub fn restore_client_backing(&self, slot: usize, state: &ClientBackingSnapshot) -> Result<(), Q3BaseError> {
        let mut inner = self.core.inner.borrow_mut();
        let Some(backing) = inner.backing.get_mut(slot) else {
            return Err(Q3BaseError::Invalid("Invalid Q3 private client backing".to_string()));
        };
        backing.source_stats = state.source_stats;
        backing.special_ammo = state.special_ammo;
        Ok(())
    }

    /// Restore an existing owned actor without reallocating its identity
    /// or body.
    pub fn adopt(&self, slot: usize, actor: OwnedActor) -> Result<EntityRef, Q3BaseError> {
        self.core.host.actors().assert_owned(&actor)?;
        let valid = slot >= MAX_CLIENTS
            && slot < ENTITYNUM_WORLD as usize
            && actor.owner() == &self.core.provider
            && self.core.record_actor(slot).is_none()
            && self.core.native_by_actor(Some(actor.id())).is_none()
            && self.core.host.bodies().read(actor.id()).is_some();
        if !valid {
            return Err(Q3BaseError::Invalid(
                "Q3 owned actor adoption requires an unused entity slot and its existing body".to_string(),
            ));
        }
        {
            let mut inner = self.core.inner.borrow_mut();
            let record = &mut inner.slots[slot];
            record.actor = Some(actor);
            record.active = true;
            record.borrowed = false;
        }
        let entity = self.core.record_entity(slot);
        entity.borrow_mut().s.number = slot as i32;
        self.core.bind_owned_services(slot);
        Ok(entity)
    }

    /// Attach an already admitted actor without creating another actor
    /// or changing its callbacks.
    pub fn attach(&self, slot: usize, actor: OwnedActor, player: bool) -> Result<EntityRef, Q3BaseError> {
        self.core.host.actors().assert_owned(&actor)?;
        {
            let mut inner = self.core.inner.borrow_mut();
            let Some(record) = inner.slots.get_mut(slot) else {
                panic!("Q3 entity {slot} outside 0..1023");
            };
            if record.actor.as_ref().is_some_and(|owned| *owned != actor) {
                return Err(Q3BaseError::Invalid(format!("Q3 slot {slot} already has an actor")));
            }
            record.actor = Some(actor);
            record.active = true;
            record.borrowed = true;
        }
        let entity = self.core.record_entity(slot);
        if player {
            let client = self.core.client_ref(slot);
            entity.borrow_mut().client = Some(client);
        }
        entity.borrow_mut().s.number = slot as i32;
        Ok(entity)
    }

    /// Activate a slot, allocating its actor.
    ///
    /// # Panics
    ///
    /// Panics when `slot` is outside `0..1023`.
    #[must_use]
    pub fn activate(&self, slot: usize) -> EntityRef {
        if slot >= MAX_GENTITIES {
            panic!("Q3 entity {slot} outside 0..1023");
        }
        self.core.ensure_actor(slot);
        self.core.inner.borrow_mut().slots[slot].active = true;
        self.core.record_entity(slot)
    }

    /// Deactivate a client slot, releasing its owned actor.
    ///
    /// # Panics
    ///
    /// Panics when `slot` is outside `0..1023`.
    pub fn deactivate_client(&self, slot: usize) {
        if slot >= MAX_GENTITIES {
            panic!("Q3 entity {slot} outside 0..1023");
        }
        let actor = self.core.record_actor(slot);
        let borrowed = self.core.inner.borrow().slots[slot].borrowed;
        if let Some(actor) = actor {
            if !borrowed {
                self.core.host.actors().release(&actor);
            }
        }
        let mut inner = self.core.inner.borrow_mut();
        inner.slots[slot].actor = None;
        inner.slots[slot].active = false;
        inner.slots[slot].borrowed = false;
    }

    /// Release an entity and reset its source metadata in place.
    ///
    /// # Panics
    ///
    /// Panics when the entity belongs to another record owner.
    pub fn release(&self, entity: EntityRef) {
        let slot = entity.borrow().slot;
        let owned = self.core.record_entity(slot);
        if !Rc::ptr_eq(&owned, &entity) {
            panic!("Q3 entity belongs to another record owner");
        }
        let actor = self.core.record_actor(slot);
        let borrowed = self.core.inner.borrow().slots[slot].borrowed;
        if let Some(actor) = actor {
            if !borrowed {
                self.core.host.actors().release(&actor);
            }
        }
        {
            let mut inner = self.core.inner.borrow_mut();
            inner.slots[slot].actor = None;
            inner.slots[slot].active = false;
            inner.slots[slot].borrowed = false;
        }
        entity.borrow_mut().reset();
    }

    /// Native entity for an actor.
    #[must_use]
    pub fn native_by_actor(&self, actor: Option<&ActorId>) -> Option<EntityRef> {
        self.core.native_by_actor(actor)
    }

    /// Native or foreign entity for an actor.
    #[must_use]
    pub fn by_actor(&self, actor: Option<&ActorId>) -> Option<EntityRef> {
        self.core.by_actor(actor)
    }

    /// Use participant for an actor.
    #[must_use]
    pub fn use_participant(&self, actor: Option<&ActorId>) -> Option<DamageParticipant> {
        self.core.use_participant(actor)
    }

    /// Damage inflictor for an actor.
    #[must_use]
    pub fn damage_inflictor(&self, actor: Option<&ActorId>) -> DamageParticipant {
        self.core.damage_inflictor(actor)
    }
}

// ---------------------------------------------------------------------------
// contracts/world.ts (unified from base/mirrors.rs)
// ---------------------------------------------------------------------------

/// Zero body state for unowned record slots.
pub const ZERO_BODY: BodyState = BodyState {
    origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
    angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
    velocity: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
    bounds: Bounds {
        min: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
        max: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
    },
    ground: None,
};

// ---------------------------------------------------------------------------
// Base-group error (unified from base/mirrors.rs)
// ---------------------------------------------------------------------------

/// Base-game failure for map/save-data inputs (donor `Error`, `RangeError`,
/// and `CommonError("drop", ...)`).
#[derive(Debug, Clone, PartialEq, Error)]
pub enum Q3BaseError {
    /// Dropped-operation failure (donor `CommonError("drop", ...)`).
    #[error("drop: {0}")]
    Drop(String),
    /// Invalid map/save/entity input (donor `Error`).
    #[error("invalid: {0}")]
    Invalid(String),
    /// Out-of-range map/save/entity input (donor `RangeError`).
    #[error("range: {0}")]
    Range(String),
}

// ---------------------------------------------------------------------------
// contracts/gameplay.ts record words (unified from base/mirrors.rs)
// ---------------------------------------------------------------------------

/// Q1 armor effect word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1ArmorEffect {
    /// Bypass armor.
    Bypass,
    /// Half effectiveness.
    HalfEffectiveness,
}

/// Q2 classic game word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2ClassicGame {
    /// Base game.
    Base,
    /// Xatrix.
    Xatrix,
    /// Rogue.
    Rogue,
    /// Capture the flag.
    Ctf,
}

/// Q2 native cause encoding (`Q2NativeCause`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q2NativeCause {
    /// Classic encoding.
    Classic {
        /// Game.
        game: Q2ClassicGame,
        /// Native value.
        value: i32,
    },
    /// Rerelease encoding.
    Rerelease {
        /// Native identifier.
        id: i32,
        /// Friendly fire.
        friendly_fire: bool,
        /// No point loss.
        no_point_loss: bool,
    },
}

/// Environment hazard word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentHazard {
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

/// Attack cause word (`AttackProvenance` cause).
#[derive(Debug, Clone, PartialEq)]
pub enum AttackCause {
    /// Quake cause.
    Q1 {
        /// Death type.
        death_type: String,
        /// Armor effect.
        armor_effect: Option<Q1ArmorEffect>,
    },
    /// Quake II cause.
    Q2 {
        /// Canonical means of death.
        means_of_death: i32,
        /// Damage flags.
        damage_flags: i32,
        /// Native encoding.
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
        hazard: EnvironmentHazard,
    },
}

/// Attack provenance (`AttackProvenance`).
#[derive(Debug, Clone, PartialEq)]
pub struct AttackProvenance {
    /// Sequence number.
    pub sequence: i32,
    /// Attack time.
    pub time: SourceTime,
    /// Attacker.
    pub attacker: Option<ActorId>,
    /// Inflictor.
    pub inflictor: Option<ActorId>,
    /// Originating projectile.
    pub originating_projectile: Option<ActorId>,
    /// Weapon item.
    pub weapon: Option<ItemId>,
    /// Weapon provider.
    pub weapon_provider: ProviderId,
    /// Damage powerup owner, when this source already applied its
    /// damage modifier.
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

/// Damage request (`DamageRequest`).
#[derive(Debug, Clone, PartialEq)]
pub struct DamageRequest {
    /// Attack provenance.
    pub attack: AttackProvenance,
    /// Target.
    pub target: ActorId,
    /// Amount.
    pub amount: f32,
    /// Knockback.
    pub knockback: f32,
    /// Direction.
    pub direction: Vec3,
    /// Point.
    pub point: Vec3,
    /// Normal.
    pub normal: Vec3,
    /// Delivery.
    pub delivery: Delivery,
}

/// Regular armor state (`RegularArmorState`).
#[derive(Debug, Clone, PartialEq)]
pub enum RegularArmorState {
    /// No armor.
    None,
    /// Quake armor.
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
        /// Protection.
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
#[derive(Debug, Clone, PartialEq)]
pub enum PoweredProtectionState {
    /// No powered protection.
    None,
    /// Screen.
    Screen {
        /// Cells.
        cells: i32,
    },
    /// Shield.
    Shield {
        /// Cells.
        cells: i32,
    },
}

/// Armor state (`ArmorState`).
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorState {
    /// Regular armor.
    pub regular: RegularArmorState,
    /// Powered protection.
    pub powered: PoweredProtectionState,
}

/// Combat state snapshot (`CombatState`).
#[derive(Debug, Clone, PartialEq)]
pub struct CombatState {
    /// Health.
    pub health: i32,
    /// Armor.
    pub armor: ArmorState,
    /// Mass.
    pub mass: i32,
    /// Whether damage is admitted.
    pub can_take_damage: bool,
    /// Invulnerable.
    pub invulnerable: bool,
    /// Source immunity to damage momentum.
    pub no_knockback: bool,
    /// Team word.
    pub team: Option<String>,
}

/// Damage mutation (`DamageMutation`).
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

/// Damage feedback word (`DamageDecision` feedback).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageFeedback {
    /// Quake II feedback.
    Q2 {
        /// Power armor saved.
        power_armor: i32,
        /// Armor saved.
        armor: i32,
        /// Blood.
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

/// Damage decision (`DamageDecision`).
#[derive(Debug, Clone, PartialEq)]
pub struct DamageDecision {
    /// Damage request.
    pub request: DamageRequest,
    /// Mutations.
    pub mutations: Vec<DamageMutation>,
    /// Applied damage.
    pub applied_damage: i32,
    /// Reaction.
    pub reaction: Reaction,
    /// Feedback.
    pub feedback: Option<DamageFeedback>,
}

/// Damage outcome (`DamageOutcome`).
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

// ---------------------------------------------------------------------------
// game/state.ts entity records (unified from base/mirrors.rs)
// ---------------------------------------------------------------------------

/// Shared game entity handle.
pub type EntityRef = Rc<RefCell<GameEntity>>;

/// Shared game client handle.
pub type ClientRef = Rc<RefCell<GameClient>>;

/// Damage participant (`DamageParticipant`/`UseParticipant`/`DamageInflictor`,
/// game/state.ts).
#[derive(Clone)]
pub enum DamageParticipant {
    /// Native entity.
    Native(EntityRef),
    /// Shared foreign actor.
    SharedActor(SharedParticipant),
}

impl std::fmt::Debug for DamageParticipant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Native(entity) => f.debug_tuple("Native").field(&entity.borrow().slot).finish(),
            Self::SharedActor(participant) => f.debug_tuple("SharedActor").field(&participant.actor).finish(),
        }
    }
}

/// Shared foreign participant (game/state.ts `shared-actor` layer).
#[derive(Clone)]
pub struct SharedParticipant {
    /// Actor.
    pub actor: ActorId,
    origin: Rc<dyn Fn() -> Option<Vec3>>,
}

impl SharedParticipant {
    /// Shared participant with an origin resolver.
    #[must_use]
    pub fn new(actor: ActorId, origin: Rc<dyn Fn() -> Option<Vec3>>) -> Self {
        Self { actor, origin }
    }

    /// Resolve the current origin, if live.
    #[must_use]
    pub fn origin(&self) -> Option<Vec3> {
        (self.origin)()
    }
}

/// Use participant (`UseParticipant`, game/state.ts).
pub type UseParticipant = DamageParticipant;

/// Damage inflictor (`DamageInflictor`, game/state.ts).
pub type DamageInflictor = DamageParticipant;

/// Actor for a participant (`useActor`, game/use-participant.ts).
#[must_use]
pub fn use_actor(participant: &DamageParticipant) -> ActorId {
    match participant {
        DamageParticipant::Native(entity) => entity.borrow().actor().id().clone(),
        DamageParticipant::SharedActor(shared) => shared.actor.clone(),
    }
}

/// Entity think callback (`EntityThink`, game/state.ts).
pub type EntityThinkCallback = Rc<dyn Fn(EntityRef)>;

/// Entity touch callback (`EntityTouch`, game/state.ts).
pub type EntityTouchCallback = Rc<dyn Fn(EntityRef, DamageParticipant, TouchContact)>;

/// Entity use callback (`EntityUse`, game/state.ts).
pub type EntityUseCallback = Rc<dyn Fn(EntityRef, Option<UseParticipant>, Option<UseParticipant>)>;

/// Entity pain callback (`EntityPain`, game/state.ts).
pub type EntityPainCallback = Rc<dyn Fn(EntityRef, DamageParticipant, i32)>;

/// Entity die callback (`EntityDie`, game/state.ts).
pub type EntityDieCallback = Rc<dyn Fn(EntityRef, DamageInflictor, DamageParticipant, i32, i32)>;

/// Source-zero `gentity_t` binding (`GameEntityBinding`, game/state.ts).
pub trait GameEntityBinding {
    /// Body binding.
    fn body(&self) -> Rc<dyn EntityBodyBinding>;
    /// Owning actor.
    fn actor(&self) -> OwnedActor;
    /// Whether the slot is live.
    fn active(&self) -> bool;
    /// Health.
    fn health(&self) -> i32;
    /// Write health.
    fn set_health(&self, value: i32);
    /// Whether damage is admitted.
    fn takes_damage(&self) -> bool;
    /// Write damage admission.
    fn set_takes_damage(&self, value: bool);
    /// Schedule the next think.
    fn schedule(&self, nextthink: i32);
    /// Run a think at a time.
    fn run_think(&self, time_milliseconds: i32);
}

/// Source-zero `gentity_t` (`GameEntity`, game/state.ts).
///
/// This mirror carries the words the base root reads and writes; the
/// sibling game-layer port owns the full record.
pub struct GameEntity {
    /// Owned-table slot.
    pub slot: usize,
    /// Session binding.
    pub binding: Rc<dyn GameEntityBinding>,
    /// Entity state words.
    pub s: EntityState,
    /// Shared collision metadata.
    pub r: EntityShared,
    /// Client record, for player slots.
    pub client: Option<ClientRef>,
    /// Game flags.
    pub flags: i32,
    /// Team name.
    pub team: Option<String>,
    /// Next teammate in the chain.
    pub teamchain: Option<EntityRef>,
    /// Team master (weak; the master owns the chain).
    pub teammaster: Option<Weak<RefCell<GameEntity>>>,
    /// Target name.
    pub targetname: Option<String>,
    nextthink_value: i32,
    /// Think callback.
    pub think: Option<EntityThinkCallback>,
    /// Touch callback.
    pub touch: Option<EntityTouchCallback>,
    /// Use callback.
    pub use_action: Option<EntityUseCallback>,
    /// Pain callback.
    pub pain: Option<EntityPainCallback>,
    /// Die callback.
    pub die: Option<EntityDieCallback>,
}

impl std::fmt::Debug for GameEntity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameEntity")
            .field("slot", &self.slot)
            .field("flags", &self.flags)
            .field("team", &self.team)
            .finish()
    }
}

impl GameEntity {
    /// Source-zero entity over a binding.
    ///
    /// # Panics
    ///
    /// Panics when `slot` is outside `0..1023`.
    #[must_use]
    pub fn new(slot: usize, binding: Rc<dyn GameEntityBinding>) -> Self {
        assert!(slot < MAX_GENTITIES, "Game entity slot outside 0..1023");
        let r = EntityShared::new(binding.body());
        Self {
            slot,
            binding,
            s: EntityState::new(),
            r,
            client: None,
            flags: 0,
            team: None,
            teamchain: None,
            teammaster: None,
            targetname: None,
            nextthink_value: 0,
            think: None,
            touch: None,
            use_action: None,
            pain: None,
            die: None,
        }
    }

    /// Whether the slot is live.
    #[must_use]
    pub fn inuse(&self) -> bool {
        self.binding.active()
    }

    /// Owning actor.
    #[must_use]
    pub fn actor(&self) -> OwnedActor {
        self.binding.actor()
    }

    /// Health.
    #[must_use]
    pub fn health(&self) -> i32 {
        self.binding.health()
    }

    /// Write health.
    pub fn set_health(&self, value: i32) {
        self.binding.set_health(value);
    }

    /// Whether damage is admitted.
    #[must_use]
    pub fn takes_damage(&self) -> bool {
        self.binding.takes_damage()
    }

    /// Write damage admission.
    pub fn set_takes_damage(&self, value: bool) {
        self.binding.set_takes_damage(value);
    }

    /// Next think time.
    #[must_use]
    pub fn nextthink(&self) -> i32 {
        self.nextthink_value
    }

    /// Write the next think time and schedule it.
    pub fn set_nextthink(&mut self, value: i32) {
        self.nextthink_value = value;
        self.binding.schedule(value);
    }

    /// Restore a think time without scheduling (save hydration).
    pub fn restore_nextthink(&mut self, value: i32) {
        self.nextthink_value = value;
    }

    /// Reset source metadata in place, keeping the slot and binding.
    pub fn reset(&mut self) {
        let binding = self.binding.clone();
        let slot = self.slot;
        *self = Self::new(slot, binding);
    }
}

impl SharedEntity for GameEntity {
    fn entity_state(&self) -> &EntityState {
        &self.s
    }

    fn shared(&self) -> &EntityShared {
        &self.r
    }
}

/// Client session words (`ClientSession`, game/state.ts, minimal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientSession {
    /// Session team.
    pub session_team: Team,
}

impl Default for ClientSession {
    fn default() -> Self {
        Self {
            session_team: Team::TeamFree,
        }
    }
}

/// Source-zero `gclient_t` (`GameClient`, game/state.ts).
///
/// This mirror carries the words the base root reads and writes; the
/// sibling game-layer port owns the full record.
pub struct GameClient {
    /// Player state.
    pub ps: PlayerState,
    /// Session.
    pub sess: ClientSession,
    /// Noclip.
    pub noclip: bool,
    /// Invulnerability time.
    pub invulnerability_time: i32,
    /// Ammunition timers.
    pub ammo_times: PlayerStateSlots,
}

impl std::fmt::Debug for GameClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameClient")
            .field("ps", &self.ps)
            .field("sess", &self.sess)
            .finish()
    }
}

impl GameClient {
    /// Source-zero client with an optional authority and ammunition
    /// store notification.
    #[must_use]
    pub fn new(
        product: Product,
        authority: Option<Rc<dyn PlayerAuthorityBinding>>,
        ammo_timer_stored: Option<Rc<dyn Fn(usize, i32)>>,
    ) -> Self {
        Self {
            ps: create_player_state(product, authority),
            sess: ClientSession::default(),
            noclip: false,
            invulnerability_time: 0,
            ammo_times: PlayerStateSlots::new(weapon_count(product) as usize, None, None, ammo_timer_stored),
        }
    }
}

// ---------------------------------------------------------------------------
// contracts/world.ts reactions (unified from base/mirrors.rs)
// ---------------------------------------------------------------------------

/// Touch contact (`TouchContact`, contracts/world.ts, minimal).
#[derive(Debug, Clone, PartialEq)]
pub struct TouchContact {
    /// Other actor.
    pub other: ActorId,
}

/// Pain reaction (`PainReaction`, contracts/world.ts, minimal).
#[derive(Debug, Clone, PartialEq)]
pub struct PainReaction {
    /// Attack provenance, if any.
    pub attack: Option<AttackProvenance>,
    /// Attacker.
    pub attacker: Option<ActorId>,
    /// Damage.
    pub damage: i32,
}

/// Death reaction (`DeathReaction`, contracts/world.ts, minimal).
#[derive(Debug, Clone, PartialEq)]
pub struct DeathReaction {
    /// Attack provenance, if any.
    pub attack: Option<AttackProvenance>,
    /// Attacker.
    pub attacker: Option<ActorId>,
    /// Damage.
    pub damage: i32,
    /// Inflictor.
    pub inflictor: Option<ActorId>,
    /// Point.
    pub point: Vec3,
}

/// Actor callbacks bound by the records (`ActorCallbacks`,
/// contracts/world.ts, minimal).
#[derive(Clone)]
pub struct ActorCallbacks {
    /// Think callback.
    pub think: Rc<dyn Fn()>,
    /// Touch callback.
    pub touch: Rc<dyn Fn(&TouchContact)>,
    /// Use callback over other and activator actors.
    pub use_action: Rc<dyn Fn(Option<ActorId>, Option<ActorId>)>,
    /// Pain callback.
    pub pain: Rc<dyn Fn(&PainReaction)>,
    /// Die callback.
    pub die: Rc<dyn Fn(&DeathReaction)>,
}

// ---------------------------------------------------------------------------
// Session combat services (unified from base/mirrors.rs)
// ---------------------------------------------------------------------------

/// Damage admission word (`admitDamage` result).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageAdmission {
    /// Continue to combat.
    Continue,
    /// Handled by the game.
    Handled,
}

/// Damage admission callback.
pub type DamageAdmissionFn = Rc<dyn Fn(&DamageRequest) -> DamageAdmission>;

/// Session actor registry services (`SessionActorRegistry`, minimal).
pub trait Q3SessionActors {
    /// Assert an actor handle is owned.
    fn assert_owned(&self, actor: &OwnedActor) -> Result<(), Q3BaseError>;
    /// Allocate an actor at a source slot.
    fn allocate_at_source(&self, provider: &ProviderId, slot: usize, definition: &str) -> OwnedActor;
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Observe actor release; returns an unobserve callback.
    fn on_release(&self, callback: Box<dyn Fn(&OwnedActor)>) -> Box<dyn Fn()>;
    /// Release an actor.
    fn release(&self, actor: &OwnedActor);
    /// Resolve an owned handle.
    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor>;
}

/// Shared body table services (`SharedBodyTable`, minimal).
pub trait Q3SessionBodies {
    /// Create a body.
    fn create(&self, actor: &OwnedActor, state: BodyState);
    /// Read a body.
    fn read(&self, actor: &ActorId) -> Option<BodyState>;
    /// Write a body.
    fn write(&self, actor: &OwnedActor, state: BodyState);
    /// Read a linked body.
    fn linked(&self, actor: &ActorId) -> Option<LinkedBody>;
    /// Link a body, optionally at a snapped origin.
    fn link(&self, actor: &OwnedActor, origin: Option<Vec3>);
    /// Unlink a body.
    fn unlink(&self, actor: &OwnedActor);
}

/// Gameplay authority services (`GameplayAuthority`, minimal).
pub trait Q3SessionCombat {
    /// Read combat state.
    fn read(&self, actor: &ActorId) -> Option<CombatState>;
    /// Create combat state with optional damage admission.
    fn create(&self, actor: &OwnedActor, initial: CombatState, admit_damage: Option<DamageAdmissionFn>);
    /// Write health.
    fn set_health(&self, actor: &OwnedActor, health: i32);
    /// Write damage admission.
    fn set_can_take_damage(&self, actor: &OwnedActor, can_take_damage: bool);
    /// Write regular armor points.
    fn set_regular_points(&self, actor: &OwnedActor, points: i32, initial: RegularArmorState);
    /// Bind damage admission.
    fn bind_damage_admission(&self, actor: &OwnedActor, admit_damage: DamageAdmissionFn);
    /// Apply a damage request.
    fn apply(&self, request: DamageRequest) -> DamageOutcome;
}

/// Inventory entry (`InventoryEntry`, minimal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryEntry {
    /// Item.
    pub item: ItemId,
    /// Count.
    pub count: i32,
    /// Capacity.
    pub capacity: i32,
}

/// Shared inventory table services (`SharedInventoryTable`, minimal).
pub trait Q3SessionInventory {
    /// Whether an inventory exists.
    fn has(&self, actor: &ActorId) -> bool;
    /// Create an inventory.
    fn create(&self, actor: &OwnedActor, entries: Vec<InventoryEntry>);
    /// Count an item.
    fn count(&self, actor: &ActorId, item: &ItemId) -> i32;
    /// Configure an item count and capacity.
    fn configure(&self, actor: &OwnedActor, item: &ItemId, count: i32, capacity: i32);
}

/// Actor callback table services (`ActorCallbackTable`, minimal).
pub trait Q3ActorCallbacks {
    /// Bind actor callbacks.
    fn bind(&self, actor: &OwnedActor, callbacks: ActorCallbacks);
}

/// Damage call record (`Q3DamageCall`, game/combat.ts).
#[derive(Clone)]
pub struct Q3DamageCall {
    /// Target.
    pub target: EntityRef,
    /// Inflictor participant.
    pub source: DamageParticipant,
    /// Attacker participant.
    pub owner: UseParticipant,
    /// Direction.
    pub direction: Option<Vec3>,
    /// Point.
    pub point: Option<Vec3>,
    /// Amount.
    pub amount: f32,
    /// Flags.
    pub flags: i32,
    /// Means of death.
    pub method_of_death: i32,
}

impl std::fmt::Debug for Q3DamageCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Q3DamageCall")
            .field("target", &self.target.borrow().slot)
            .field("amount", &self.amount)
            .field("flags", &self.flags)
            .field("method_of_death", &self.method_of_death)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// game/entities.ts entity pool view (unified from base/mirrors.rs)
// ---------------------------------------------------------------------------

/// Entity pool view (`EntityPool`, game/entities.ts, minimal).
pub trait Q3EntityPool {
    /// Entity count.
    fn num_entities(&self) -> usize;
    /// Entity at an index.
    fn entity_at(&self, index: usize) -> EntityRef;
}

/// Shared entity pool handle.
pub type EntityPoolRef = Rc<dyn Q3EntityPool>;
