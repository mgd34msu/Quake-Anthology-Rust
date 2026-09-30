//! Quake III base: records.
//!
//! Donor provenance: `src/content/q3/base/records.ts`.

use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::math::{vec3, Vec3};
use std::cell::RefCell;
use std::rc::{Rc, Weak};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::state::{MAX_CLIENTS, MAX_GENTITIES};
use crate::q3::base::mirrors::*;
use crate::q3::base::shared::definitions::*;
use crate::q3::base::shared::entity_shared::*;
use crate::q3::base::shared::player_state::*;

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
            return q3_weapon_items().iter().fold(0, |bits, weapon| {
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
            for weapon in q3_weapon_items() {
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
