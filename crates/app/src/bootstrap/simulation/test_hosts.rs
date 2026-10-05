//! Shared fake Q1/Q2 engine hosts for simulation unit tests.
//!
//! Test-only support (`#[cfg(test)]` at the `mod` gate): HashMap-backed
//! actor, body, combat, inventory, and callback tables plus canned trace
//! scripts and clocks, so placement, monster, and execution tests can build
//! real [`Q1EntityServices`] and [`Q2GameServices`] values without an engine.
//!
//! The fakes implement the content host traits honestly for the surface the
//! tests exercise (allocation, liveness, saved-actor round trips, body
//! link snapshots, combat trait writes, inventory counts, trace scripts).
//! Damage application always reports a stale target (the grapple tests use
//! the same limitation); tests never route damage through these hosts.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_content::contract::{ArmorState, InventoryEntry, ItemId, PoweredProtectionState, RegularArmorState};
use qa_content::q1::foundation::entity_services::Q1EntityServices;
use qa_content::q1::foundation::gameplay::{
    ActorObservation as Q1ActorObservation, BodyAttachment as Q1BodyAttachment, BodyState as Q1BodyState,
    CombatState as Q1CombatState, CombatTraits as Q1CombatTraits, DamageOutcome as Q1DamageOutcome,
    DamageRequest as Q1DamageRequest, LinkedBody as Q1LinkedBody, SourceSlot,
};
use qa_content::q1::foundation::host::{
    Q1ActorCallbackTable, Q1Contents, Q1DamageAdjustHook, Q1FoundationHost, Q1GameplayAuthority, Q1PusherStatus,
    Q1PusherStep, Q1SessionActorRegistry, Q1SharedBodyTable, Q1SharedInventoryTable,
};
use qa_content::q1::foundation::types::{Q1Edition, Q1FoundationOptions, Q1PrecacheProgram, Q1Trace};
use qa_content::q1::Q1Error;
use qa_content::q2::foundation::host::{
    Q2Edition, Q2GameOptions, Q2GameServices, Q2Mode, Q2Motion, Q2Solid, Q2TraceRequest,
};
use qa_content::q2::foundation::host::{Q2FoundationHost, Q2LandmarkCarry, Q2PlayerViewState, Q2PresentationEvent};
use qa_content::q2::support::contracts::{
    ActorObservation as Q2ActorObservation, BodyAttachment as Q2BodyAttachment, BodyState as Q2BodyState,
    CombatState as Q2CombatState, CombatTraitChanges, DamageOutcome as Q2DamageOutcome,
    DamageRequest as Q2DamageRequest, LinkedBody as Q2LinkedBody, PowerArmorCells, Q2BspPlane, Q2TraceFields,
    TraceContact as Q2TraceContact, TraceFamily, TraceHit as Q2TraceHit, TraceResult as Q2TraceResult,
    TransitionIntent,
};
use qa_content::q2::support::tables::{
    Q2ActorRegistry, Q2BodyTable, Q2CallbackTable, Q2CombatAuthority, Q2InventoryTable,
};
use qa_core::identity::{ActorId, IdentityOwner, OwnedActor, ProviderId, SavedActorId};
use qa_core::math::{Bounds, Vec3};

fn up() -> Vec3 {
    Vec3 { x: 0.0, y: 0.0, z: 1.0 }
}

fn zero_bounds() -> Bounds {
    Bounds {
        min: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
        max: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
    }
}

fn absolute_bounds(origin: Vec3, bounds: Bounds) -> Bounds {
    Bounds {
        min: Vec3 {
            x: origin.x + bounds.min.x,
            y: origin.y + bounds.min.y,
            z: origin.z + bounds.min.z,
        },
        max: Vec3 {
            x: origin.x + bounds.max.x,
            y: origin.y + bounds.max.y,
            z: origin.z + bounds.max.z,
        },
    }
}

/// Generational actor registry shared by the Q1 and Q2 fakes.
struct SimActorsInner {
    identities: IdentityOwner,
    live: HashSet<ActorId>,
    owned: HashMap<ActorId, OwnedActor>,
    definitions: HashMap<ActorId, String>,
    slots: HashMap<ActorId, u32>,
    next_slot: u32,
}

/// Cloneable fake actor registry handle.
#[derive(Clone)]
pub struct SimActors(Rc<RefCell<SimActorsInner>>);

impl SimActors {
    /// Fresh registry rooted at a test identity owner.
    pub fn new(name: &str) -> Self {
        Self(Rc::new(RefCell::new(SimActorsInner {
            identities: IdentityOwner::create(name).expect("test owner"),
            live: HashSet::new(),
            owned: HashMap::new(),
            definitions: HashMap::new(),
            slots: HashMap::new(),
            next_slot: 1,
        })))
    }

    /// Mint a live actor with a dynamic slot.
    pub fn mint(&self, owner: &ProviderId, definition: &str) -> OwnedActor {
        let slot = self.0.borrow().next_slot;
        self.mint_at(owner, slot, definition)
    }

    /// Mint a live actor at an explicit source slot.
    pub fn mint_at(&self, owner: &ProviderId, slot: u32, definition: &str) -> OwnedActor {
        let mut inner = self.0.borrow_mut();
        let id = inner.identities.actor(slot, 0);
        let owned = inner.identities.owned_actor(&id, owner.clone()).expect("fake owner");
        inner.next_slot = inner.next_slot.max(slot + 1);
        inner.live.insert(id.clone());
        inner.owned.insert(id.clone(), owned.clone());
        inner.definitions.insert(id.clone(), definition.to_string());
        inner.slots.insert(id, slot);
        owned
    }

    fn resolve(&self, saved: &SavedActorId) -> Option<OwnedActor> {
        let inner = self.0.borrow();
        let id = inner.identities.actor(saved.slot, saved.generation);
        if inner.live.contains(&id) {
            inner.owned.get(&id).cloned()
        } else {
            None
        }
    }
}

impl Q1SessionActorRegistry for SimActors {
    fn allocate_at_source(
        &mut self,
        owner: &ProviderId,
        source_slot: u32,
        definition: &str,
    ) -> Result<OwnedActor, Q1Error> {
        Ok(self.mint_at(owner, source_slot, definition))
    }

    fn assert_owned(&self, actor: &OwnedActor) -> Result<(), Q1Error> {
        let inner = self.0.borrow();
        match inner.owned.get(actor.id()) {
            Some(known) if known.owner() == actor.owner() => Ok(()),
            _ => Err(Q1Error::Message("test actor is not owned".to_string())),
        }
    }

    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
        let inner = self.0.borrow();
        if inner.live.contains(actor) {
            inner.owned.get(actor).cloned()
        } else {
            None
        }
    }

    fn is_live(&self, actor: &ActorId) -> bool {
        self.0.borrow().live.contains(actor)
    }

    fn observations(&self) -> Vec<Q1ActorObservation> {
        let inner = self.0.borrow();
        inner
            .live
            .iter()
            .filter_map(|id| {
                let owned = inner.owned.get(id)?;
                Some(Q1ActorObservation {
                    id: id.clone(),
                    owner: owned.owner().clone(),
                    definition: inner.definitions.get(id).cloned().unwrap_or_default(),
                })
            })
            .collect()
    }

    fn source_of(&self, actor: &ActorId) -> Option<SourceSlot> {
        let inner = self.0.borrow();
        let owned = inner.owned.get(actor)?;
        Some(SourceSlot {
            provider: owned.owner().clone(),
            slot: inner.slots.get(actor).copied().unwrap_or(0),
        })
    }

    fn release(&mut self, actor: &OwnedActor) -> Result<(), Q1Error> {
        self.0.borrow_mut().live.remove(actor.id());
        Ok(())
    }

    fn resolve_saved(&self, saved: &SavedActorId) -> Option<OwnedActor> {
        self.resolve(saved)
    }

    fn reference_saved(&mut self, saved: &SavedActorId) -> ActorId {
        self.0.borrow().identities.actor(saved.slot, saved.generation)
    }
}

impl Q2ActorRegistry for SimActors {
    fn allocate(&mut self, owner: &ProviderId, definition: &str) -> OwnedActor {
        self.mint(owner, definition)
    }

    fn allocate_at_source(&mut self, owner: &ProviderId, source_slot: u32, definition: &str) -> OwnedActor {
        self.mint_at(owner, source_slot, definition)
    }

    fn source_of(&self, actor: &ActorId) -> Option<(ProviderId, u32)> {
        let inner = self.0.borrow();
        let owned = inner.owned.get(actor)?;
        Some((owned.owner().clone(), inner.slots.get(actor).copied().unwrap_or(0)))
    }

    fn release(&mut self, actor: &OwnedActor) {
        self.0.borrow_mut().live.remove(actor.id());
    }

    fn is_live(&self, actor: &ActorId) -> bool {
        self.0.borrow().live.contains(actor)
    }

    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
        let inner = self.0.borrow();
        if inner.live.contains(actor) {
            inner.owned.get(actor).cloned()
        } else {
            None
        }
    }

    fn observations(&self) -> Vec<Q2ActorObservation> {
        let inner = self.0.borrow();
        inner
            .live
            .iter()
            .filter_map(|id| {
                let owned = inner.owned.get(id)?;
                Some(Q2ActorObservation {
                    id: id.clone(),
                    owner: owned.owner().clone(),
                    definition: inner.definitions.get(id).cloned().unwrap_or_default(),
                })
            })
            .collect()
    }

    fn resolve_saved(&self, saved: SavedActorId) -> Option<OwnedActor> {
        self.resolve(&saved)
    }

    fn reference_saved(&self, saved: SavedActorId) -> ActorId {
        self.0.borrow().identities.actor(saved.slot, saved.generation)
    }

    fn assert_owned(&self, actor: &OwnedActor) {
        debug_assert!(
            self.0.borrow().owned.contains_key(actor.id()),
            "test actor is not owned"
        );
    }
}

/// Neutral body record converted into each family's state shape.
#[derive(Debug, Clone)]
struct NeutralBody {
    origin: Vec3,
    angles: Vec3,
    velocity: Vec3,
    bounds: Bounds,
    ground: Option<ActorId>,
    absolute: Bounds,
    link_count: u64,
    linked: bool,
}

impl NeutralBody {
    fn relink(&mut self) {
        self.absolute = absolute_bounds(self.origin, self.bounds);
        self.link_count += 1;
        self.linked = true;
    }
}

struct SimBodiesInner {
    records: HashMap<ActorId, NeutralBody>,
    q1_attachments: HashMap<ActorId, Q1BodyAttachment>,
    q2_attachments: HashMap<ActorId, Q2BodyAttachment>,
}

/// Cloneable fake body table handle.
#[derive(Clone)]
pub struct SimBodies(Rc<RefCell<SimBodiesInner>>);

impl SimBodies {
    /// Fresh table.
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(SimBodiesInner {
            records: HashMap::new(),
            q1_attachments: HashMap::new(),
            q2_attachments: HashMap::new(),
        })))
    }

    /// Admit and link a body directly, returning its snapshot.
    pub fn admit_linked(&self, actor: &ActorId, origin: Vec3, bounds: Bounds) {
        let mut inner = self.0.borrow_mut();
        let mut record = NeutralBody {
            origin,
            angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            velocity: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            bounds,
            ground: None,
            absolute: zero_bounds(),
            link_count: 0,
            linked: false,
        };
        record.relink();
        inner.records.insert(actor.clone(), record);
    }
}

impl Default for SimBodies {
    fn default() -> Self {
        Self::new()
    }
}

impl Q1SharedBodyTable for SimBodies {
    fn create(&mut self, actor: &OwnedActor, initial: &Q1BodyState) -> Result<(), Q1Error> {
        self.0.borrow_mut().records.insert(
            actor.id().clone(),
            NeutralBody {
                origin: initial.origin,
                angles: initial.angles,
                velocity: initial.velocity,
                bounds: initial.bounds,
                ground: initial.ground.clone(),
                absolute: absolute_bounds(initial.origin, initial.bounds),
                link_count: 0,
                linked: false,
            },
        );
        Ok(())
    }

    fn read(&self, actor: &ActorId) -> Option<Q1BodyState> {
        self.0.borrow().records.get(actor).map(|record| Q1BodyState {
            origin: record.origin,
            angles: record.angles,
            velocity: record.velocity,
            bounds: record.bounds,
            ground: record.ground.clone(),
        })
    }

    fn write(&mut self, actor: &OwnedActor, state: &Q1BodyState) -> Result<(), Q1Error> {
        match self.0.borrow_mut().records.get_mut(actor.id()) {
            Some(record) => {
                record.origin = state.origin;
                record.angles = state.angles;
                record.velocity = state.velocity;
                record.bounds = state.bounds;
                record.ground.clone_from(&state.ground);
                Ok(())
            }
            None => Err(Q1Error::Message("test body is missing".to_string())),
        }
    }

    fn link(&mut self, actor: &OwnedActor) -> Result<(), Q1Error> {
        match self.0.borrow_mut().records.get_mut(actor.id()) {
            Some(record) => {
                record.relink();
                Ok(())
            }
            None => Err(Q1Error::Message("test body is missing".to_string())),
        }
    }

    fn linked(&self, actor: &ActorId) -> Option<Q1LinkedBody> {
        self.0
            .borrow()
            .records
            .get(actor)
            .filter(|record| record.linked)
            .map(|record| Q1LinkedBody {
                actor: actor.clone(),
                state: Q1BodyState {
                    origin: record.origin,
                    angles: record.angles,
                    velocity: record.velocity,
                    bounds: record.bounds,
                    ground: record.ground.clone(),
                },
                absolute_bounds: record.absolute,
                link_count: record.link_count,
            })
    }

    fn attach(&mut self, actor: &OwnedActor, attachment: &Q1BodyAttachment) -> Result<(), Q1Error> {
        self.0
            .borrow_mut()
            .q1_attachments
            .insert(actor.id().clone(), attachment.clone());
        Ok(())
    }

    fn detach(&mut self, actor: &OwnedActor) -> Result<(), Q1Error> {
        self.0.borrow_mut().q1_attachments.remove(actor.id());
        Ok(())
    }
}

impl Q2BodyTable for SimBodies {
    fn create(&mut self, actor: &OwnedActor, initial: &Q2BodyState) {
        self.0.borrow_mut().records.insert(
            actor.id().clone(),
            NeutralBody {
                origin: initial.origin,
                angles: initial.angles,
                velocity: initial.velocity,
                bounds: initial.bounds,
                ground: initial.ground.clone(),
                absolute: absolute_bounds(initial.origin, initial.bounds),
                link_count: 0,
                linked: false,
            },
        );
    }

    fn read(&self, actor: &ActorId) -> Option<Q2BodyState> {
        self.0.borrow().records.get(actor).map(|record| Q2BodyState {
            origin: record.origin,
            angles: record.angles,
            velocity: record.velocity,
            bounds: record.bounds,
            ground: record.ground.clone(),
        })
    }

    fn write(&mut self, actor: &OwnedActor, state: &Q2BodyState) {
        if let Some(record) = self.0.borrow_mut().records.get_mut(actor.id()) {
            record.origin = state.origin;
            record.angles = state.angles;
            record.velocity = state.velocity;
            record.bounds = state.bounds;
            record.ground.clone_from(&state.ground);
        }
    }

    fn attach(&mut self, actor: &OwnedActor, attachment: &Q2BodyAttachment) {
        self.0
            .borrow_mut()
            .q2_attachments
            .insert(actor.id().clone(), attachment.clone());
    }

    fn detach(&mut self, actor: &OwnedActor) {
        self.0.borrow_mut().q2_attachments.remove(actor.id());
    }

    fn attachment(&self, actor: &ActorId) -> Option<Q2BodyAttachment> {
        self.0.borrow().q2_attachments.get(actor).cloned()
    }

    fn linked(&self, actor: &ActorId) -> Option<Q2LinkedBody> {
        self.0
            .borrow()
            .records
            .get(actor)
            .filter(|record| record.linked)
            .map(|record| Q2LinkedBody {
                actor: actor.clone(),
                state: Q2BodyState {
                    origin: record.origin,
                    angles: record.angles,
                    velocity: record.velocity,
                    bounds: record.bounds,
                    ground: record.ground.clone(),
                },
                absolute_bounds: record.absolute,
                link_count: record.link_count,
            })
    }

    fn link(&mut self, actor: &OwnedActor, origin: Option<Vec3>) {
        if let Some(record) = self.0.borrow_mut().records.get_mut(actor.id()) {
            if let Some(origin) = origin {
                record.origin = origin;
            }
            record.relink();
        }
    }

    fn unlink(&mut self, actor: &OwnedActor) {
        if let Some(record) = self.0.borrow_mut().records.get_mut(actor.id()) {
            record.linked = false;
        }
    }
}

/// Neutral combat record converted into each family's state shape.
#[derive(Debug, Clone)]
struct NeutralCombat {
    health: f64,
    armor: ArmorState,
    mass: f64,
    can_take_damage: bool,
    invulnerable: bool,
    no_knockback: Option<bool>,
    team: Option<String>,
}

/// Cloneable fake combat authority handle.
#[derive(Clone)]
pub struct SimCombat(Rc<RefCell<HashMap<ActorId, NeutralCombat>>>);

impl SimCombat {
    /// Fresh table.
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(HashMap::new())))
    }
}

impl Default for SimCombat {
    fn default() -> Self {
        Self::new()
    }
}

/// Set regular armor points, preserving the current armor kind.
fn set_points(regular: &mut RegularArmorState, points: f64) {
    match regular {
        RegularArmorState::None => {
            *regular = RegularArmorState::Source { points, item: None };
        }
        RegularArmorState::Q1 { points: current, .. }
        | RegularArmorState::Q2 { points: current, .. }
        | RegularArmorState::Q3 { points: current, .. }
        | RegularArmorState::Source { points: current, .. } => {
            *current = points;
        }
    }
}

impl Q1GameplayAuthority for SimCombat {
    fn create(&mut self, actor: &OwnedActor, initial: &Q1CombatState) -> Result<(), Q1Error> {
        self.0.borrow_mut().insert(
            actor.id().clone(),
            NeutralCombat {
                health: initial.health,
                armor: initial.armor.clone(),
                mass: initial.mass,
                can_take_damage: initial.can_take_damage,
                invulnerable: initial.invulnerable,
                no_knockback: initial.no_knockback,
                team: initial.team.clone(),
            },
        );
        Ok(())
    }

    fn read(&self, actor: &ActorId) -> Option<Q1CombatState> {
        self.0.borrow().get(actor).map(|record| Q1CombatState {
            health: record.health,
            armor: record.armor.clone(),
            mass: record.mass,
            can_take_damage: record.can_take_damage,
            invulnerable: record.invulnerable,
            no_knockback: record.no_knockback,
            team: record.team.clone(),
        })
    }

    fn set_health(&mut self, actor: &OwnedActor, health: f64) -> Result<(), Q1Error> {
        if let Some(record) = self.0.borrow_mut().get_mut(actor.id()) {
            record.health = health;
        }
        Ok(())
    }

    fn set_armor(&mut self, actor: &OwnedActor, armor: &ArmorState) -> Result<(), Q1Error> {
        if let Some(record) = self.0.borrow_mut().get_mut(actor.id()) {
            record.armor = armor.clone();
        }
        Ok(())
    }

    fn set_traits(&mut self, actor: &OwnedActor, traits: Q1CombatTraits) -> Result<(), Q1Error> {
        if let Some(record) = self.0.borrow_mut().get_mut(actor.id()) {
            record.can_take_damage = traits.can_take_damage;
            record.mass = traits.mass;
            record.invulnerable = traits.invulnerable;
            record.team.clone_from(&traits.team);
            record.no_knockback = traits.no_knockback;
        }
        Ok(())
    }

    fn set_regular_armor(&mut self, actor: &OwnedActor, regular: &RegularArmorState) -> Result<(), Q1Error> {
        if let Some(record) = self.0.borrow_mut().get_mut(actor.id()) {
            record.armor.regular = regular.clone();
        }
        Ok(())
    }

    fn set_regular_points(&mut self, actor: &OwnedActor, points: f64) -> Result<(), Q1Error> {
        if let Some(record) = self.0.borrow_mut().get_mut(actor.id()) {
            set_points(&mut record.armor.regular, points);
        }
        Ok(())
    }

    fn bind_damage_adjustment(&mut self, _actor: &OwnedActor, _adjust: Q1DamageAdjustHook) {}

    fn apply(&mut self, input: &Q1DamageRequest) -> Q1DamageOutcome {
        Q1DamageOutcome::StaleTarget { request: input.clone() }
    }
}

impl Q2CombatAuthority for SimCombat {
    fn create(&mut self, actor: &OwnedActor, initial: &Q2CombatState) {
        self.0.borrow_mut().insert(
            actor.id().clone(),
            NeutralCombat {
                health: initial.health,
                armor: initial.armor.clone(),
                mass: initial.mass,
                can_take_damage: initial.can_take_damage,
                invulnerable: initial.invulnerable,
                no_knockback: Some(initial.no_knockback),
                team: initial.team.clone(),
            },
        );
    }

    fn read(&self, actor: &ActorId) -> Option<Q2CombatState> {
        self.0.borrow().get(actor).map(|record| Q2CombatState {
            health: record.health,
            armor: record.armor.clone(),
            mass: record.mass,
            can_take_damage: record.can_take_damage,
            invulnerable: record.invulnerable,
            no_knockback: record.no_knockback.unwrap_or(false),
            team: record.team.clone(),
        })
    }

    fn set_health(&mut self, actor: &OwnedActor, health: f64) {
        if let Some(record) = self.0.borrow_mut().get_mut(actor.id()) {
            record.health = health;
        }
    }

    fn set_armor(&mut self, actor: &OwnedActor, armor: &ArmorState) {
        if let Some(record) = self.0.borrow_mut().get_mut(actor.id()) {
            record.armor = armor.clone();
        }
    }

    fn set_regular_points(&mut self, actor: &OwnedActor, points: f64, _initial: Option<&RegularArmorState>) {
        if let Some(record) = self.0.borrow_mut().get_mut(actor.id()) {
            set_points(&mut record.armor.regular, points);
        }
    }

    fn set_regular_armor(&mut self, actor: &OwnedActor, regular: &RegularArmorState) {
        if let Some(record) = self.0.borrow_mut().get_mut(actor.id()) {
            record.armor.regular = regular.clone();
        }
    }

    fn set_powered_protection(&mut self, actor: &OwnedActor, powered: &PoweredProtectionState) {
        if let Some(record) = self.0.borrow_mut().get_mut(actor.id()) {
            record.armor.powered = powered.clone();
        }
    }

    fn set_traits(&mut self, actor: &OwnedActor, changes: &CombatTraitChanges) {
        if let Some(record) = self.0.borrow_mut().get_mut(actor.id()) {
            if let Some(can_take_damage) = changes.can_take_damage {
                record.can_take_damage = can_take_damage;
            }
            if let Some(mass) = changes.mass {
                record.mass = mass;
            }
            if let Some(invulnerable) = changes.invulnerable {
                record.invulnerable = invulnerable;
            }
            if let Some(team) = changes.team.clone() {
                record.team = team;
            }
            if let Some(no_knockback) = changes.no_knockback {
                record.no_knockback = Some(no_knockback);
            }
        }
    }

    fn bind_power_armor_cells(&mut self, _actor: &OwnedActor, _cells: Box<dyn PowerArmorCells>) {}

    fn apply(&mut self, input: &Q2DamageRequest) -> Q2DamageOutcome {
        Q2DamageOutcome::StaleTarget { request: input.clone() }
    }
}

/// Cloneable fake inventory table handle.
#[derive(Clone)]
pub struct SimInventory(Rc<RefCell<SimInventoryInner>>);

struct SimInventoryInner {
    entries: HashMap<ActorId, Vec<InventoryEntry>>,
    bound: HashSet<ActorId>,
}

impl SimInventory {
    /// Fresh table.
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(SimInventoryInner {
            entries: HashMap::new(),
            bound: HashSet::new(),
        })))
    }

    fn upsert(entries: &mut Vec<InventoryEntry>, item: &ItemId, count: f64) -> f64 {
        match entries.iter_mut().find(|entry| &entry.item == item) {
            Some(entry) => {
                entry.count += count;
                entry.count
            }
            None => {
                entries.push(InventoryEntry {
                    item: item.clone(),
                    count,
                    capacity: f64::INFINITY,
                    count_policy: None,
                });
                count
            }
        }
    }
}

impl Default for SimInventory {
    fn default() -> Self {
        Self::new()
    }
}

impl Q1SharedInventoryTable for SimInventory {
    fn create(&mut self, actor: &OwnedActor, entries: &[InventoryEntry]) -> Result<(), Q1Error> {
        let mut inner = self.0.borrow_mut();
        inner.entries.insert(actor.id().clone(), entries.to_vec());
        inner.bound.insert(actor.id().clone());
        Ok(())
    }

    fn entries(&self, actor: &ActorId) -> Vec<InventoryEntry> {
        let inner = self.0.borrow();
        if inner.bound.contains(actor) {
            inner.entries.get(actor).cloned().unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    fn has(&self, actor: &ActorId) -> bool {
        self.0.borrow().bound.contains(actor)
    }

    fn count(&self, actor: &ActorId, item: &ItemId) -> f64 {
        let inner = self.0.borrow();
        if !inner.bound.contains(actor) {
            return 0.0;
        }
        inner
            .entries
            .get(actor)
            .and_then(|entries| entries.iter().find(|entry| &entry.item == item))
            .map_or(0.0, |entry| entry.count)
    }

    fn consume(&mut self, actor: &OwnedActor, item: &ItemId, count: f64) -> bool {
        let mut inner = self.0.borrow_mut();
        let Some(entries) = inner.entries.get_mut(actor.id()) else {
            return false;
        };
        let Some(entry) = entries.iter_mut().find(|entry| &entry.item == item) else {
            return false;
        };
        if entry.count < count {
            return false;
        }
        entry.count -= count;
        true
    }

    fn give(&mut self, actor: &OwnedActor, item: &ItemId, count: f64) -> f64 {
        let mut inner = self.0.borrow_mut();
        let entries = inner.entries.entry(actor.id().clone()).or_default();
        Self::upsert(entries, item, count)
    }

    fn configure(&mut self, actor: &OwnedActor, entry: &InventoryEntry) -> Result<(), Q1Error> {
        let mut inner = self.0.borrow_mut();
        let entries = inner.entries.entry(actor.id().clone()).or_default();
        match entries.iter_mut().find(|slot| slot.item == entry.item) {
            Some(slot) => *slot = entry.clone(),
            None => entries.push(entry.clone()),
        }
        Ok(())
    }

    fn adjust_source_counter(&mut self, actor: &OwnedActor, item: &ItemId, delta: f64) -> Result<f64, Q1Error> {
        let mut inner = self.0.borrow_mut();
        let Some(entries) = inner.entries.get_mut(actor.id()) else {
            return Ok(0.0);
        };
        let Some(entry) = entries.iter_mut().find(|entry| &entry.item == item) else {
            return Ok(0.0);
        };
        entry.count += delta;
        Ok(entry.count)
    }
}

impl Q2InventoryTable for SimInventory {
    fn create(&mut self, actor: &OwnedActor, entries: &[InventoryEntry]) {
        let mut inner = self.0.borrow_mut();
        inner.entries.insert(actor.id().clone(), entries.to_vec());
        inner.bound.insert(actor.id().clone());
    }

    fn entries(&self, actor: &ActorId) -> Vec<InventoryEntry> {
        let inner = self.0.borrow();
        if inner.bound.contains(actor) {
            inner.entries.get(actor).cloned().unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    fn has(&self, actor: &ActorId) -> bool {
        self.0.borrow().bound.contains(actor)
    }

    fn count(&self, actor: &ActorId, item: &ItemId) -> f64 {
        let inner = self.0.borrow();
        if !inner.bound.contains(actor) {
            return 0.0;
        }
        inner
            .entries
            .get(actor)
            .and_then(|entries| entries.iter().find(|entry| &entry.item == item))
            .map_or(0.0, |entry| entry.count)
    }

    fn consume(&mut self, actor: &OwnedActor, item: &ItemId, count: f64) -> bool {
        let mut inner = self.0.borrow_mut();
        let Some(entries) = inner.entries.get_mut(actor.id()) else {
            return false;
        };
        let Some(entry) = entries.iter_mut().find(|entry| &entry.item == item) else {
            return false;
        };
        if entry.count < count {
            return false;
        }
        entry.count -= count;
        true
    }

    fn give(&mut self, actor: &OwnedActor, item: &ItemId, count: f64) -> f64 {
        let mut inner = self.0.borrow_mut();
        let entries = inner.entries.entry(actor.id().clone()).or_default();
        Self::upsert(entries, item, count)
    }

    fn configure(&mut self, actor: &OwnedActor, entry: &InventoryEntry) {
        let mut inner = self.0.borrow_mut();
        let entries = inner.entries.entry(actor.id().clone()).or_default();
        match entries.iter_mut().find(|slot| slot.item == entry.item) {
            Some(slot) => *slot = entry.clone(),
            None => entries.push(entry.clone()),
        }
    }

    fn adjust_source_counter(&mut self, actor: &OwnedActor, item: &ItemId, delta: f64) -> f64 {
        let mut inner = self.0.borrow_mut();
        let Some(entries) = inner.entries.get_mut(actor.id()) else {
            return 0.0;
        };
        let Some(entry) = entries.iter_mut().find(|entry| &entry.item == item) else {
            return 0.0;
        };
        entry.count += delta;
        entry.count
    }
}

/// Cloneable fake callback table handle.
#[derive(Clone)]
pub struct SimCallbacks(Rc<RefCell<HashSet<ActorId>>>);

impl SimCallbacks {
    /// Fresh table.
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(HashSet::new())))
    }
}

impl Default for SimCallbacks {
    fn default() -> Self {
        Self::new()
    }
}

impl Q1ActorCallbackTable for SimCallbacks {
    fn bind(&mut self, actor: &OwnedActor) {
        self.0.borrow_mut().insert(actor.id().clone());
    }

    fn unbind(&mut self, actor: &OwnedActor) {
        self.0.borrow_mut().remove(actor.id());
    }

    fn is_bound(&self, actor: &ActorId) -> bool {
        self.0.borrow().contains(actor)
    }
}

impl Q2CallbackTable for SimCallbacks {
    fn bind(&mut self, actor: &OwnedActor) {
        self.0.borrow_mut().insert(actor.id().clone());
    }

    fn unbind(&mut self, actor: &ActorId) {
        self.0.borrow_mut().remove(actor);
    }

    fn is_bound(&self, actor: &ActorId) -> bool {
        self.0.borrow().contains(actor)
    }

    fn forward_use(&mut self, _actor: &OwnedActor, _other: Option<&ActorId>, _activator: Option<&ActorId>) {}
}

/// Shared handles into a fake Q1 game.
#[allow(dead_code)]
pub struct Q1Handles {
    /// Actor registry.
    pub actors: SimActors,
    /// Body table.
    pub bodies: SimBodies,
    /// Combat authority.
    pub combat: SimCombat,
    /// Inventory table.
    pub inventory: SimInventory,
    /// Callback table.
    pub callbacks: SimCallbacks,
}

/// Classic Q1 game options for tests.
pub fn test_q1_options() -> Q1FoundationOptions {
    Q1FoundationOptions {
        provider: None,
        precache_program: Some(Q1PrecacheProgram::Id1),
        edition: Q1Edition::Classic,
        physics_edition: None,
        skill: 1,
        deathmatch: 0,
        coop: false,
        campaign: ProviderId::new("q1", "campaign"),
        combat_provider: ProviderId::new("q1", "combat"),
        movement_provider: ProviderId::new("q1", "movement"),
        inventory_provider: ProviderId::new("q1", "inventory"),
        gravity: 800.0,
        max_clients: Some(4),
        no_exit: None,
        teamplay: None,
        aim_threshold: None,
    }
}

static TEST_OWNER_SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn unique_owner(stem: &str) -> String {
    let serial = TEST_OWNER_SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{stem}-{serial}")
}

/// Build a Q1 game over fake tables; the trace hook misses by default and
/// tests may replace `game.host.trace` with a script.
pub fn test_q1_game() -> (Q1EntityServices, Q1Handles) {
    test_q1_game_with_options(test_q1_options())
}

/// Shared Q1 game with caller-supplied options.
pub fn test_q1_game_with_options(options: Q1FoundationOptions) -> (Q1EntityServices, Q1Handles) {
    let actors = SimActors::new(&unique_owner("sim-q1-test"));
    let bodies = SimBodies::new();
    let callbacks = SimCallbacks::new();
    let combat = SimCombat::new();
    let inventory = SimInventory::new();
    let host = Q1FoundationHost {
        actors: Box::new(actors.clone()),
        bodies: Box::new(bodies.clone()),
        callbacks: Box::new(callbacks.clone()),
        combat: Box::new(combat.clone()),
        inventory: Box::new(inventory.clone()),
        original_pickups: None,
        punch_angles: None,
        weapon_behavior: None,
        register_entity: None,
        source_damage_modifier: None,
        source_damage_powerup_owner: None,
        random: Box::new(|| 0.5),
        trace: Box::new(|request| Q1Trace {
            fraction: 1.0,
            end: request.end,
            normal: up(),
            actor: None,
            start_solid: false,
            all_solid: false,
            sky: false,
            in_open: true,
            in_water: false,
        }),
        contents: Box::new(|_| Q1Contents::Empty),
        walk_move: Box::new(|_, _, _| true),
        change_yaw: Box::new(|_| {}),
        move_to_goal: Box::new(|_, _, _, _| {}),
        check_bottom: Box::new(|_| true),
        schedule_think: Box::new(|_, _| {}),
        cancel_think: Box::new(|_| {}),
        emit: Box::new(|_| {}),
        transition: Box::new(|_| {}),
        players: Box::new(Vec::new),
        check_client: Box::new(|_| None),
        classname: Box::new(|_| String::new()),
        powerup: Box::new(|_, _, _| {}),
        step_pusher: Box::new(|actor, _| Q1PusherStep {
            actor: actor.clone(),
            status: Q1PusherStatus::Moved,
            moved: Vec::new(),
        }),
        weapon_impact: None,
        weapon_volume: None,
        monster_target: None,
        set_gravity: None,
        control_player: None,
        source_target: None,
        powerup_expires: None,
        source_damage_multiplier: None,
        is_bot: None,
    };
    let game = Q1EntityServices::new(host, options).expect("test game");
    let handles = Q1Handles {
        actors,
        bodies,
        combat,
        inventory,
        callbacks,
    };
    (game, handles)
}

/// Shared scriptable Q2 trace hook.
pub type TraceScript = Rc<RefCell<Box<dyn FnMut(&Q2TraceRequest) -> Q2TraceResult>>>;

/// Miss-everything Q2 trace script.
pub fn miss_trace() -> TraceScript {
    Rc::new(RefCell::new(Box::new(|request| Q2TraceResult {
        fraction: 1.0,
        end: request.end,
        start_solid: false,
        all_solid: false,
        contact: Q2TraceContact::None,
        hit: Q2TraceHit::None,
        family: TraceFamily::Q2(Q2TraceFields {
            contents: 0,
            surface: None,
            source_plane: Q2BspPlane {
                normal: up(),
                distance: 0.0,
                plane_type: 0,
                signbits: 0,
            },
            secondary: None,
        }),
    })))
}

/// Fake Q2 engine host over shared tables, a scriptable trace, and a clock.
pub struct FakeQ2Host {
    actors: SimActors,
    bodies: SimBodies,
    callbacks: SimCallbacks,
    combat: SimCombat,
    inventory: SimInventory,
    now: Rc<RefCell<f64>>,
    frame_seconds: f64,
    gravity: f64,
    trace: TraceScript,
    nearby: Vec<ActorId>,
    players: Vec<ActorId>,
    world: ActorId,
    emitted: Rc<RefCell<Vec<Q2PresentationEvent>>>,
}

impl Q2FoundationHost for FakeQ2Host {
    fn actors(&mut self) -> &mut dyn Q2ActorRegistry {
        &mut self.actors
    }

    fn bodies(&mut self) -> &mut dyn Q2BodyTable {
        &mut self.bodies
    }

    fn callbacks(&mut self) -> &mut dyn Q2CallbackTable {
        &mut self.callbacks
    }

    fn combat(&mut self) -> &mut dyn Q2CombatAuthority {
        &mut self.combat
    }

    fn inventory(&mut self) -> &mut dyn Q2InventoryTable {
        &mut self.inventory
    }

    fn now(&self) -> f64 {
        *self.now.borrow()
    }

    fn frame_seconds(&self) -> f64 {
        self.frame_seconds
    }

    fn gravity(&self) -> f64 {
        self.gravity
    }

    fn random(&mut self) -> f64 {
        0.5
    }

    fn schedule(&mut self, _actor: &OwnedActor, _due_seconds: Option<f64>) {}

    fn touch_triggers(&mut self, _actor: &OwnedActor) {}

    fn trace(&mut self, request: &Q2TraceRequest) -> Q2TraceResult {
        (self.trace.borrow_mut())(request)
    }

    fn point_contents(&mut self, _point: Vec3) -> i32 {
        0
    }

    fn in_pvs(&mut self, _first: Vec3, _second: Vec3) -> bool {
        true
    }

    fn in_phs(&mut self, _first: Vec3, _second: Vec3) -> bool {
        true
    }

    fn areas_connected(&mut self, _first: Vec3, _second: Vec3) -> bool {
        true
    }

    fn nearby(&mut self, _origin: Vec3, _radius: f64) -> Vec<ActorId> {
        self.nearby.clone()
    }

    fn players(&mut self) -> Vec<ActorId> {
        self.players.clone()
    }

    fn world_actor(&mut self) -> ActorId {
        self.world.clone()
    }

    fn is_player(&mut self, _actor: &ActorId) -> bool {
        false
    }

    fn is_monster(&mut self, _actor: &ActorId) -> bool {
        false
    }

    fn inline_model_bounds(&mut self, _model: i32) -> Bounds {
        zero_bounds()
    }

    fn set_solid(&mut self, _actor: &OwnedActor, _solid: Q2Solid, _model: Option<i32>) {}

    fn set_motion(&mut self, _motion: &Q2Motion) {}

    fn set_area_portal(&mut self, _portal: i32, _open: bool) {}

    fn emit(&mut self, event: Q2PresentationEvent) {
        self.emitted.borrow_mut().push(event);
    }

    fn player_view_state(&mut self, _player: &ActorId) -> Option<Q2PlayerViewState> {
        None
    }

    fn key_consumed(&mut self, _player: &ActorId) {}

    fn prepare_level_change(&mut self, _map: &str, _landmark: Option<&Q2LandmarkCarry>, _server_flags: i32) {}

    fn transition(&mut self, _intent: TransitionIntent) {}

    fn diagnostic(&mut self, _message: &str) {}
}

/// Shared handles into a fake Q2 game.
#[allow(dead_code)]
pub struct Q2Handles {
    /// Actor registry.
    pub actors: SimActors,
    /// Body table.
    pub bodies: SimBodies,
    /// Combat authority.
    pub combat: SimCombat,
    /// Inventory table.
    pub inventory: SimInventory,
    /// Callback table.
    pub callbacks: SimCallbacks,
    /// Scriptable trace hook.
    pub trace: TraceScript,
    /// Scriptable clock.
    pub now: Rc<RefCell<f64>>,
    /// Emitted presentation events.
    pub emitted: Rc<RefCell<Vec<Q2PresentationEvent>>>,
    /// World actor.
    pub world: ActorId,
}

/// Classic Q2 game options for tests.
pub fn test_q2_options() -> Q2GameOptions {
    Q2GameOptions {
        edition: Q2Edition::Classic,
        map_name: "test".to_string(),
        skill: 1,
        mode: Q2Mode::Singleplayer,
        deathmatch_flags: 0,
        max_clients: 4,
        provider: ProviderId::new("q2", "test"),
        damage_powerup_owner: None,
        source_damage_modifier: None,
        campaign: ProviderId::new("q2", "campaign"),
        combat_provider: ProviderId::new("q2", "combat"),
        inventory_provider: ProviderId::new("q2", "inventory"),
        movement_provider: ProviderId::new("q2", "movement"),
    }
}

/// Build a Q2 game over fake tables with a miss-everything trace script.
pub fn test_q2_game() -> (Q2GameServices, Q2Handles) {
    let actors = SimActors::new(&unique_owner("sim-q2-test"));
    let world = actors.mint(&ProviderId::new("q2", "world"), "worldspawn").id().clone();
    let now = Rc::new(RefCell::new(0.0));
    let trace = miss_trace();
    let emitted = Rc::new(RefCell::new(Vec::new()));
    let host = FakeQ2Host {
        actors: actors.clone(),
        bodies: SimBodies::new(),
        callbacks: SimCallbacks::new(),
        combat: SimCombat::new(),
        inventory: SimInventory::new(),
        now: now.clone(),
        frame_seconds: 0.1,
        gravity: 800.0,
        trace: trace.clone(),
        nearby: Vec::new(),
        players: Vec::new(),
        world: world.clone(),
        emitted: emitted.clone(),
    };
    let bodies = host.bodies.clone();
    let callbacks = host.callbacks.clone();
    let combat = host.combat.clone();
    let inventory = host.inventory.clone();
    let game = Q2GameServices::new(Box::new(host), test_q2_options(), Vec::new());
    let handles = Q2Handles {
        actors,
        bodies,
        combat,
        inventory,
        callbacks,
        trace,
        now,
        emitted,
        world,
    };
    (game, handles)
}
