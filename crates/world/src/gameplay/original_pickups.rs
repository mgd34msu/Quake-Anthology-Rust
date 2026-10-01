//! Original pickup rules and shared admission: the map retains its complete
//! touch continuation; resource bindings select original recipient code.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/gameplay/original-pickups.ts`
//! with contract shapes from `src/contracts/original-pickups.ts`.
//!
//! Rust adaptations: the donor's `Promise` branch in `run_source` has no
//! equivalent here; executors run synchronously while the touch scope is
//! held. Reference comparisons on registry handles become structural
//! equality on generational ids. Double grant consumption panics (a logic
//! error); data errors travel as [`WorldError::BadPickup`].

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::time::SourceTime;

use crate::combat::{ItemId, PoweredProtection, RegularArmor};
use crate::registry::ActorRegistry;
use crate::WorldError;

/// Protection channel: armor state key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtectionChannel {
    /// Regular armor.
    Regular,
    /// Powered protection.
    Powered,
}

impl ProtectionChannel {
    fn name(self) -> &'static str {
        match self {
            ProtectionChannel::Regular => "regular",
            ProtectionChannel::Powered => "powered",
        }
    }
}

/// Resource a pickup writes to by default.
#[derive(Debug, Clone, PartialEq)]
pub enum PickupResource {
    /// Protection channel.
    Protection {
        /// Channel.
        channel: ProtectionChannel,
    },
    /// Inventory item.
    Inventory {
        /// Item.
        item: ItemId,
    },
}

/// Inventory write fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryWriteFields {
    /// Count only.
    Count,
    /// Capacity only.
    Capacity,
    /// Count and capacity.
    CountAndCapacity,
}

/// One resource write of a pickup rule.
#[derive(Debug, Clone, PartialEq)]
pub enum PickupWrite {
    /// Protection write.
    Protection {
        /// Channel.
        channel: ProtectionChannel,
    },
    /// Inventory write.
    Inventory {
        /// Item.
        item: ItemId,
        /// Fields.
        fields: InventoryWriteFields,
    },
}

/// Write-set key: `protection:{channel}` or `inventory:{item}`.
#[must_use]
pub fn pickup_write_key(write: &PickupWrite) -> String {
    match write {
        PickupWrite::Protection { channel } => format!("protection:{}", channel.name()),
        PickupWrite::Inventory { item, .. } => format!("inventory:{item}"),
    }
}

/// Pickup count selection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PickupCount {
    /// Default count.
    Default,
    /// Override amount.
    Override(f64),
}

/// Cargo entry kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickupCargoKind {
    /// Counter cargo.
    Counter,
    /// Weapon cargo (count must be 1).
    Weapon,
}

/// One cargo row.
#[derive(Debug, Clone, PartialEq)]
pub struct PickupCargoEntry {
    /// Entry kind.
    pub kind: PickupCargoKind,
    /// Item.
    pub item: ItemId,
    /// Count.
    pub count: f64,
}

/// Grant coupling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginalPickupGrant {
    /// Map-coupled grant (owns objectives).
    MapCoupled,
    /// Source effect without inventory/protection grant.
    SourceEffect,
}

/// Original pickup offer.
#[derive(Debug, Clone, PartialEq)]
pub struct OriginalPickupOffer {
    /// Recipient actor.
    pub recipient: ActorId,
    /// Pickup actor.
    pub pickup: ActorId,
    /// Source provider.
    pub source: ProviderId,
    /// Offered item.
    pub item: ItemId,
    /// Default resource.
    pub default_resource: Option<PickupResource>,
    /// Count selection.
    pub count: PickupCount,
    /// Dropped (not map-placed).
    pub dropped: bool,
    /// Offer time.
    pub time: SourceTime,
    /// Cargo rows.
    pub cargo: Option<Vec<PickupCargoEntry>>,
    /// Grant coupling.
    pub grant: Option<OriginalPickupGrant>,
}

/// Rule decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginalPickupDecision {
    /// Accepted.
    Accepted,
    /// Refused.
    Refused,
}

/// Settled touch outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginalPickupOutcome {
    /// Accepted.
    Accepted,
    /// Refused.
    Refused,
    /// Scope went stale.
    Stale,
}

impl From<OriginalPickupDecision> for OriginalPickupOutcome {
    fn from(decision: OriginalPickupDecision) -> Self {
        match decision {
            OriginalPickupDecision::Accepted => OriginalPickupOutcome::Accepted,
            OriginalPickupDecision::Refused => OriginalPickupOutcome::Refused,
        }
    }
}

/// Before/after values of one protection channel.
#[derive(Debug, Clone, PartialEq)]
pub struct ProtectionChannelChange<T> {
    /// Value before.
    pub before: T,
    /// Value after.
    pub after: T,
}

/// One committed protection store across owned channels.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProtectionStoreChange {
    /// Regular channel change.
    pub regular: Option<ProtectionChannelChange<RegularArmor>>,
    /// Powered channel change.
    pub powered: Option<ProtectionChannelChange<PoweredProtection>>,
}

/// Reports one committed store; never repeats the write.
pub trait ProtectionObserver {
    /// Observe a committed store.
    fn stored(&self, change: &ProtectionStoreChange);
}

/// Execution context of a rule `take`.
pub trait OriginalPickupExecution {
    /// Declared writes.
    fn writes(&self) -> &[PickupWrite];
    /// Whether the binding is still current.
    fn current(&self) -> bool;
    /// Store a protection change.
    fn stored(&self, change: &ProtectionStoreChange) -> Result<(), WorldError>;
}

/// Original pickup rule.
pub trait OriginalPickupRule {
    /// Rule id.
    fn id(&self) -> &str;
    /// Offered items.
    fn offered(&self) -> &[ItemId];
    /// Declared writes (nonempty).
    fn writes(&self) -> &[PickupWrite];
    /// Attempt the grant.
    fn take(
        &self,
        offer: &OriginalPickupOffer,
        execution: &dyn OriginalPickupExecution,
    ) -> Result<OriginalPickupDecision, WorldError>;
}

/// Captured rule: validated copy retaining its operation.
#[derive(Clone)]
pub struct CapturedOriginalPickupRule {
    /// Original operation.
    pub operation: Rc<dyn OriginalPickupRule>,
    /// Rule id.
    pub id: String,
    /// Offered items.
    pub offered: Vec<ItemId>,
    /// Declared writes.
    pub writes: Vec<PickupWrite>,
}

/// Validate and capture rule operations.
pub fn capture_original_pickup_rules(
    rules: Vec<Rc<dyn OriginalPickupRule>>,
) -> Result<Vec<CapturedOriginalPickupRule>, WorldError> {
    let mut ids = HashSet::new();
    let mut offered = HashSet::new();
    let mut captured = Vec::with_capacity(rules.len());
    for rule in rules {
        if rule.id().is_empty() || !ids.insert(rule.id().to_string()) || rule.offered().is_empty() {
            return Err(WorldError::BadPickup(
                "Original pickup rules require unique IDs and offered items".to_string(),
            ));
        }
        for item in rule.offered() {
            if !offered.insert(item.clone()) {
                return Err(WorldError::BadPickup(format!(
                    "Ambiguous original pickup rule for {item}"
                )));
            }
        }
        if rule.writes().is_empty() {
            return Err(WorldError::BadPickup(
                "Original pickup requires a nonempty write set".to_string(),
            ));
        }
        let writes = rule.writes().to_vec();
        let keys: HashSet<String> = writes.iter().map(pickup_write_key).collect();
        if keys.len() != writes.len() {
            return Err(WorldError::BadPickup(
                "Original pickup has duplicate resource writes".to_string(),
            ));
        }
        captured.push(CapturedOriginalPickupRule {
            operation: Rc::clone(&rule),
            id: rule.id().to_string(),
            offered: rule.offered().to_vec(),
            writes,
        });
    }
    Ok(captured)
}

/// One accepted resource binding.
#[derive(Clone)]
pub struct CurrentOriginalPickup {
    /// Owning provider.
    pub owner: ProviderId,
    /// Rule operation.
    pub operation: Rc<dyn OriginalPickupRule>,
    /// Captured rule.
    pub captured: Rc<CapturedOriginalPickupRule>,
    /// Matched write.
    pub write: PickupWrite,
    /// Freshness check.
    pub current: Rc<dyn Fn() -> bool>,
}

/// Resolution of an offer against resource bindings.
#[derive(Clone, Default)]
pub struct OriginalPickupResolution {
    /// Accepted bindings.
    pub matches: Vec<CurrentOriginalPickup>,
    /// Whether the default resource blocks the primary path.
    pub blocks_primary: bool,
}

/// Actor handle resolution for pickup scopes.
pub trait OriginalPickupActors {
    /// Resolve a live handle to its owned authority.
    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor>;
}

impl OriginalPickupActors for ActorRegistry {
    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
        ActorRegistry::resolve_owned(self, actor)
    }
}

impl OriginalPickupActors for Rc<RefCell<ActorRegistry>> {
    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
        self.borrow().resolve_owned(actor)
    }
}

impl<T: OriginalPickupActors> OriginalPickupActors for &T {
    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
        (*self).resolve_owned(actor)
    }
}

/// Combat side of pickup resolution.
pub trait OriginalPickupCombat {
    /// Resolve an offer against protection bindings.
    fn resolve_pickup(&self, recipient: &OwnedActor, offer: &OriginalPickupOffer) -> OriginalPickupResolution;
    /// Run an operation with the owner's protection observer.
    fn with_pickup_protection<T>(
        &self,
        recipient: &OwnedActor,
        owner: &ProviderId,
        operation: &dyn Fn(&dyn ProtectionObserver) -> T,
    ) -> T;
}

/// Inventory side of pickup resolution.
pub trait OriginalPickupInventory {
    /// Resolve an offer against inventory bindings.
    fn resolve_pickup(&self, recipient: &OwnedActor, offer: &OriginalPickupOffer) -> OriginalPickupResolution;
}

/// Map touch continuation sharing the item scope.
pub trait OriginalPickupContinuation {
    /// Map touch eligibility.
    fn eligible(&self) -> bool {
        true
    }
    /// Run the original grant.
    fn original(&self) -> bool;
    /// Settle a taken/refused attempt while the touch scope is held.
    fn complete(&self, taken: bool);
}

/// Source selection for an offer.
pub enum SourcePickupSelection<'a> {
    /// Original recipient code handles the offer.
    Original,
    /// Offer is blocked.
    Blocked,
    /// Scope went stale.
    Stale,
    /// Replacement grant owns the offer.
    Replacement {
        /// Freshness check.
        current: Box<dyn Fn() -> bool + 'a>,
        /// Consume the grant once.
        grant: Box<dyn Fn() -> Result<OriginalPickupOutcome, WorldError> + 'a>,
    },
}

struct Selection<'a> {
    selection: SourcePickupSelection<'a>,
    selectable: bool,
    selection_current: Option<Rc<dyn Fn() -> bool + 'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Consumption {
    Live,
    Removing,
    Consumed,
}

struct PickupScopeState {
    recipient: OwnedActor,
    pickup: OwnedActor,
    request: OriginalPickupOffer,
    open: bool,
    consumption: Consumption,
    accepted: bool,
}

/// Map lifecycle handle: only the qualified executor retires its pickup.
pub struct SourcePickupLifetime<'a, A: OriginalPickupActors> {
    scope: Option<Rc<RefCell<PickupScopeState>>>,
    actors: &'a A,
    is_current: Option<Rc<dyn Fn() -> bool + 'a>>,
    selection_current: Option<Rc<dyn Fn() -> bool + 'a>>,
    selectable: bool,
}

impl<A: OriginalPickupActors> SourcePickupLifetime<'_, A> {
    /// Retire the pickup through `remove`, verifying the exact scope.
    pub fn consume_pickup(&self, remove: &dyn Fn()) -> Result<(), WorldError> {
        let Some(scope) = &self.scope else {
            return Err(WorldError::BadPickup("Stale pickup cannot be consumed".to_string()));
        };
        let state = scope.borrow();
        let current = self.is_current.as_ref().is_some_and(|check| check());
        let selected = self.selectable && self.selection_current.as_ref().is_none_or(|check| check());
        if !state.accepted || state.consumption != Consumption::Live || !current || !selected {
            return Err(WorldError::BadPickup(
                "Original pickup consumption lost its source scope".to_string(),
            ));
        }
        drop(state);
        scope.borrow_mut().consumption = Consumption::Removing;
        remove();
        let state = scope.borrow();
        if !state.open
            || self.actors.resolve_owned(state.recipient.id()) != Some(state.recipient.clone())
            || self.actors.resolve_owned(state.pickup.id()).is_some()
        {
            return Err(WorldError::BadPickup(
                "Original pickup consumption did not retire its exact pickup".to_string(),
            ));
        }
        drop(state);
        scope.borrow_mut().consumption = Consumption::Consumed;
        if self.selection_current.as_ref().is_some_and(|check| !check()) {
            return Err(WorldError::BadPickup(
                "Original pickup consumption changed its recipient binding".to_string(),
            ));
        }
        Ok(())
    }
}

type EligibilityGate = dyn Fn(&OriginalPickupOffer) -> bool;

/// Shared admission over actor, combat, and inventory hosts.
pub struct SharedOriginalPickupAdmission<A, C, I> {
    actors: A,
    combat: C,
    inventory: I,
    eligible: Option<Box<EligibilityGate>>,
    touching: RefCell<HashSet<ActorId>>,
}

impl<A: OriginalPickupActors, C: OriginalPickupCombat, I: OriginalPickupInventory>
    SharedOriginalPickupAdmission<A, C, I>
{
    /// Build admission over the three hosts with an optional eligibility gate.
    pub fn new(actors: A, combat: C, inventory: I, eligible: Option<Box<EligibilityGate>>) -> Self {
        Self {
            actors,
            combat,
            inventory,
            eligible,
            touching: RefCell::new(HashSet::new()),
        }
    }

    /// Fail when a pickup executes (checkpoints require idle admission).
    pub fn assert_idle(&self) -> Result<(), WorldError> {
        if self.touching.borrow().is_empty() {
            Ok(())
        } else {
            Err(WorldError::BadPickup(
                "Cannot checkpoint during pickup execution".to_string(),
            ))
        }
    }

    fn open(&self, offer: OriginalPickupOffer) -> Result<Option<Rc<RefCell<PickupScopeState>>>, WorldError> {
        let (Some(recipient), Some(pickup)) = (
            self.actors.resolve_owned(&offer.recipient),
            self.actors.resolve_owned(&offer.pickup),
        ) else {
            return Ok(None);
        };
        if self.touching.borrow().contains(pickup.id()) {
            return Ok(None);
        }
        let finite_time = offer.time.as_seconds_f64().is_finite();
        let finite_count = !matches!(offer.count, PickupCount::Override(amount) if !amount.is_finite());
        if !finite_time || !finite_count {
            return Err(WorldError::BadPickup(
                "Pickup time and count must be finite".to_string(),
            ));
        }
        if let Some(cargo) = &offer.cargo {
            let items: HashSet<&ItemId> = cargo.iter().map(|row| &row.item).collect();
            let valid = items.len() == cargo.len()
                && cargo
                    .iter()
                    .all(|row| row.count.is_finite() && (row.kind != PickupCargoKind::Weapon || row.count == 1.0));
            if !valid {
                return Err(WorldError::BadPickup(
                    "Pickup cargo has invalid counts or duplicate items".to_string(),
                ));
            }
        }
        self.touching.borrow_mut().insert(pickup.id().clone());
        Ok(Some(Rc::new(RefCell::new(PickupScopeState {
            recipient,
            pickup,
            request: offer,
            open: true,
            consumption: Consumption::Live,
            accepted: false,
        }))))
    }

    fn is_current(&self, scope: &Rc<RefCell<PickupScopeState>>) -> bool {
        let state = scope.borrow();
        if !state.open || self.actors.resolve_owned(state.recipient.id()) != Some(state.recipient.clone()) {
            return false;
        }
        if state.consumption == Consumption::Consumed {
            self.actors.resolve_owned(state.pickup.id()).is_none()
        } else {
            self.actors.resolve_owned(state.pickup.id()) == Some(state.pickup.clone())
        }
    }

    fn close(&self, scope: &Rc<RefCell<PickupScopeState>>) {
        scope.borrow_mut().open = false;
        let pickup = scope.borrow().pickup.id().clone();
        self.touching.borrow_mut().remove(&pickup);
    }

    fn selection<'s>(&'s self, scope: &Rc<RefCell<PickupScopeState>>) -> Result<Selection<'s>, WorldError> {
        let request = scope.borrow().request.clone();
        let allowed = self.eligible.as_ref().is_none_or(|gate| gate(&request));
        if !self.is_current(scope) {
            return Ok(Selection {
                selection: SourcePickupSelection::Stale,
                selectable: false,
                selection_current: None,
            });
        }
        if !allowed {
            return Ok(Selection {
                selection: SourcePickupSelection::Blocked,
                selectable: false,
                selection_current: None,
            });
        }
        if request.grant == Some(OriginalPickupGrant::SourceEffect) {
            if scope.borrow().pickup.owner() != &request.source {
                return Err(WorldError::BadPickup(
                    "Original pickup effect belongs to another source owner".to_string(),
                ));
            }
            return Ok(Selection {
                selection: SourcePickupSelection::Original,
                selectable: true,
                selection_current: None,
            });
        }
        let recipient = scope.borrow().recipient.clone();
        let armor = self.combat.resolve_pickup(&recipient, &request);
        let items = self.inventory.resolve_pickup(&recipient, &request);
        let matches: Vec<CurrentOriginalPickup> = armor
            .matches
            .iter()
            .cloned()
            .chain(items.matches.iter().cloned())
            .collect();
        let Some(selected) = matches.first() else {
            let blocked = armor.blocks_primary || items.blocks_primary;
            return Ok(Selection {
                selection: if blocked {
                    SourcePickupSelection::Blocked
                } else {
                    SourcePickupSelection::Original
                },
                selectable: !blocked,
                selection_current: None,
            });
        };
        if matches
            .iter()
            .any(|match_| match_.owner != selected.owner || !Rc::ptr_eq(&match_.operation, &selected.operation))
        {
            return Err(WorldError::BadPickup(format!(
                "Multiple original pickup owners accept {}",
                request.item
            )));
        }
        let declared = selected.captured.writes.clone();
        let keys: HashSet<String> = matches.iter().map(|match_| pickup_write_key(&match_.write)).collect();
        let complete = keys.len() == declared.len()
            && declared.iter().all(|write| keys.contains(&pickup_write_key(write)))
            && matches.iter().all(|match_| match_.captured.writes == declared);
        if !complete {
            return Err(WorldError::BadPickup(format!(
                "Original pickup {} has an incomplete or changed resource write set",
                request.item
            )));
        }
        if request.grant == Some(OriginalPickupGrant::MapCoupled) {
            return Err(WorldError::BadPickup(format!(
                "Original pickup replacement for {} requires its map lifecycle",
                request.item
            )));
        }
        let scope_check = Rc::clone(scope);
        let matches_check = matches.clone();
        let current: Rc<dyn Fn() -> bool + 's> =
            Rc::new(move || self.is_current(&scope_check) && matches_check.iter().all(|match_| (match_.current)()));
        let grant_current = Rc::clone(&current);
        let meta_current = Rc::clone(&current);
        let grant_scope = Rc::clone(scope);
        let grant_owner = selected.owner.clone();
        let grant_captured = Rc::clone(&selected.captured);
        let grant_request = request.clone();
        let grant_recipient = recipient.clone();
        let used = Rc::new(Cell::new(false));
        Ok(Selection {
            selection: SourcePickupSelection::Replacement {
                current: Box::new(move || grant_current()),
                grant: Box::new(move || {
                    if used.get() {
                        panic!("Original source pickup grant already consumed");
                    }
                    used.set(true);
                    if !current() {
                        return Ok(OriginalPickupOutcome::Stale);
                    }
                    struct TakeExecution<'e> {
                        declared: &'e [PickupWrite],
                        open: &'e Cell<bool>,
                        live: Rc<dyn Fn() -> bool + 'e>,
                        stores: &'e dyn ProtectionObserver,
                    }
                    impl OriginalPickupExecution for TakeExecution<'_> {
                        fn writes(&self) -> &[PickupWrite] {
                            self.declared
                        }
                        fn current(&self) -> bool {
                            (self.live)()
                        }
                        fn stored(&self, change: &ProtectionStoreChange) -> Result<(), WorldError> {
                            if !self.open.get() {
                                return Err(WorldError::BadPickup("Original pickup observer is closed".to_string()));
                            }
                            if !(self.live)() {
                                return Err(WorldError::BadPickup(
                                    "Original pickup resource binding is no longer current".to_string(),
                                ));
                            }
                            let regular_declared = self.declared.iter().any(|write| {
                                matches!(
                                    write,
                                    PickupWrite::Protection {
                                        channel: ProtectionChannel::Regular
                                    }
                                )
                            });
                            let powered_declared = self.declared.iter().any(|write| {
                                matches!(
                                    write,
                                    PickupWrite::Protection {
                                        channel: ProtectionChannel::Powered
                                    }
                                )
                            });
                            if change.regular.is_some() && !regular_declared
                                || change.powered.is_some() && !powered_declared
                            {
                                return Err(WorldError::BadPickup(
                                    "Original pickup changed undeclared protection".to_string(),
                                ));
                            }
                            self.stores.stored(change);
                            Ok(())
                        }
                    }
                    let open = Cell::new(true);
                    struct Closer<'c> {
                        open: &'c Cell<bool>,
                    }
                    impl Drop for Closer<'_> {
                        fn drop(&mut self) {
                            self.open.set(false);
                        }
                    }
                    let _closer = Closer { open: &open };
                    let open_ref = &open;
                    let outcome = self
                        .combat
                        .with_pickup_protection(&grant_recipient, &grant_owner, &|stores| {
                            let execution = TakeExecution {
                                declared: &grant_captured.writes,
                                open: open_ref,
                                live: {
                                    let live = Rc::clone(&current);
                                    Rc::new(move || open_ref.get() && live())
                                },
                                stores,
                            };
                            grant_captured.operation.take(&grant_request, &execution)
                        });
                    let outcome = match outcome {
                        Ok(decision) => OriginalPickupOutcome::from(decision),
                        Err(error) => return Err(error),
                    };
                    if !current() {
                        return Ok(OriginalPickupOutcome::Stale);
                    }
                    grant_scope.borrow_mut().accepted = outcome == OriginalPickupOutcome::Accepted;
                    Ok(outcome)
                }),
            },
            selectable: true,
            selection_current: Some(meta_current),
        })
    }

    /// Run an offer through selection with its map lifetime held open.
    /// Synchronous: executors settle before the touch scope closes.
    pub fn run_source<Output>(
        &self,
        offer: OriginalPickupOffer,
        execute: &dyn Fn(SourcePickupSelection<'_>, SourcePickupLifetime<'_, A>) -> Output,
    ) -> Result<Output, WorldError> {
        struct Guard<'g, A: OriginalPickupActors, C: OriginalPickupCombat, I: OriginalPickupInventory> {
            admission: &'g SharedOriginalPickupAdmission<A, C, I>,
            scope: Option<Rc<RefCell<PickupScopeState>>>,
        }
        impl<A: OriginalPickupActors, C: OriginalPickupCombat, I: OriginalPickupInventory> Drop for Guard<'_, A, C, I> {
            fn drop(&mut self) {
                if let Some(scope) = &self.scope {
                    self.admission.close(scope);
                }
            }
        }
        let scope = match self.open(offer)? {
            Some(scope) => scope,
            None => {
                return Ok(execute(
                    SourcePickupSelection::Stale,
                    SourcePickupLifetime {
                        scope: None,
                        actors: &self.actors,
                        is_current: None,
                        selection_current: None,
                        selectable: false,
                    },
                ));
            }
        };
        let guard = Guard {
            admission: self,
            scope: Some(Rc::clone(&scope)),
        };
        let selected = self.selection(&scope)?;
        if matches!(selected.selection, SourcePickupSelection::Original) {
            scope.borrow_mut().accepted = true;
        }
        let scope_check = Rc::clone(&scope);
        let actors = &self.actors;
        let lifetime = SourcePickupLifetime {
            scope: Some(Rc::clone(&scope)),
            actors: &self.actors,
            is_current: Some(Rc::new(move || {
                let state = scope_check.borrow();
                if !state.open || actors.resolve_owned(state.recipient.id()) != Some(state.recipient.clone()) {
                    return false;
                }
                if state.consumption == Consumption::Consumed {
                    actors.resolve_owned(state.pickup.id()).is_none()
                } else {
                    actors.resolve_owned(state.pickup.id()) == Some(state.pickup.clone())
                }
            })),
            selection_current: selected.selection_current,
            selectable: selected.selectable,
        };
        let result = execute(selected.selection, lifetime);
        drop(guard);
        Ok(result)
    }

    /// Touch an offer through a map continuation.
    pub fn touch(
        &self,
        offer: OriginalPickupOffer,
        continuation: &dyn OriginalPickupContinuation,
    ) -> Result<OriginalPickupOutcome, WorldError> {
        let scope = match self.open(offer)? {
            Some(scope) => scope,
            None => return Ok(OriginalPickupOutcome::Stale),
        };
        struct Guard<'g, A: OriginalPickupActors, C: OriginalPickupCombat, I: OriginalPickupInventory> {
            admission: &'g SharedOriginalPickupAdmission<A, C, I>,
            scope: Rc<RefCell<PickupScopeState>>,
        }
        impl<A: OriginalPickupActors, C: OriginalPickupCombat, I: OriginalPickupInventory> Drop for Guard<'_, A, C, I> {
            fn drop(&mut self) {
                self.admission.close(&self.scope);
            }
        }
        let _guard = Guard {
            admission: self,
            scope: Rc::clone(&scope),
        };
        if !continuation.eligible() {
            return Ok(OriginalPickupOutcome::Refused);
        }
        if !self.is_current(&scope) {
            return Ok(OriginalPickupOutcome::Stale);
        }
        let selected = self.selection(&scope)?;
        let (outcome, replacement_current) = match &selected.selection {
            SourcePickupSelection::Replacement { current, grant } => {
                let outcome = grant()?;
                let current = current();
                (outcome, Some(current))
            }
            SourcePickupSelection::Original => {
                let outcome = if continuation.original() {
                    OriginalPickupOutcome::Accepted
                } else {
                    OriginalPickupOutcome::Refused
                };
                (outcome, None)
            }
            _ => (OriginalPickupOutcome::Refused, None),
        };
        if outcome == OriginalPickupOutcome::Stale || !self.is_current(&scope) || replacement_current == Some(false) {
            return Ok(OriginalPickupOutcome::Stale);
        }
        continuation.complete(outcome == OriginalPickupOutcome::Accepted);
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell as StdRefCell;

    use qa_core::identity::IdentityOwner;

    struct StubActors {
        registry: StdRefCell<ActorRegistry>,
    }

    impl StubActors {
        fn new() -> Self {
            Self {
                registry: StdRefCell::new(
                    ActorRegistry::new(IdentityOwner::create("pickup-test").unwrap(), 8).unwrap(),
                ),
            }
        }

        fn spawn(&self, owner: &ProviderId, definition: &str) -> OwnedActor {
            self.registry.borrow_mut().allocate(owner.clone(), definition).unwrap()
        }

        fn retire(&self, actor: &OwnedActor) {
            self.registry.borrow_mut().release(actor).unwrap();
        }
    }

    impl OriginalPickupActors for StubActors {
        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            self.registry.borrow().resolve_owned(actor)
        }
    }

    struct StubRule {
        id: String,
        offered: Vec<ItemId>,
        writes: Vec<PickupWrite>,
        decision: OriginalPickupDecision,
    }

    impl OriginalPickupRule for StubRule {
        fn id(&self) -> &str {
            &self.id
        }
        fn offered(&self) -> &[ItemId] {
            &self.offered
        }
        fn writes(&self) -> &[PickupWrite] {
            &self.writes
        }
        fn take(
            &self,
            _offer: &OriginalPickupOffer,
            _execution: &dyn OriginalPickupExecution,
        ) -> Result<OriginalPickupDecision, WorldError> {
            Ok(self.decision)
        }
    }

    struct StubCombat {
        matches: StdRefCell<Vec<CurrentOriginalPickup>>,
        stored: StdRefCell<Vec<ProtectionStoreChange>>,
    }

    impl OriginalPickupCombat for StubCombat {
        fn resolve_pickup(&self, _recipient: &OwnedActor, _offer: &OriginalPickupOffer) -> OriginalPickupResolution {
            OriginalPickupResolution {
                matches: self.matches.borrow().clone(),
                blocks_primary: false,
            }
        }

        fn with_pickup_protection<T>(
            &self,
            _recipient: &OwnedActor,
            _owner: &ProviderId,
            operation: &dyn Fn(&dyn ProtectionObserver) -> T,
        ) -> T {
            struct Recorder<'a> {
                stored: &'a StdRefCell<Vec<ProtectionStoreChange>>,
            }
            impl ProtectionObserver for Recorder<'_> {
                fn stored(&self, change: &ProtectionStoreChange) {
                    self.stored.borrow_mut().push(change.clone());
                }
            }
            operation(&Recorder { stored: &self.stored })
        }
    }

    struct StubInventory {
        matches: Vec<CurrentOriginalPickup>,
        blocks_primary: bool,
    }

    impl OriginalPickupInventory for StubInventory {
        fn resolve_pickup(&self, _recipient: &OwnedActor, _offer: &OriginalPickupOffer) -> OriginalPickupResolution {
            OriginalPickupResolution {
                matches: self.matches.clone(),
                blocks_primary: self.blocks_primary,
            }
        }
    }

    struct StubContinuation {
        eligible: bool,
        original: bool,
        completed: StdRefCell<Vec<bool>>,
    }

    impl OriginalPickupContinuation for StubContinuation {
        fn eligible(&self) -> bool {
            self.eligible
        }
        fn original(&self) -> bool {
            self.original
        }
        fn complete(&self, taken: bool) {
            self.completed.borrow_mut().push(taken);
        }
    }

    fn provider() -> ProviderId {
        ProviderId::new("q1", "game")
    }

    fn offer(recipient: &ActorId, pickup: &ActorId) -> OriginalPickupOffer {
        OriginalPickupOffer {
            recipient: recipient.clone(),
            pickup: pickup.clone(),
            source: provider(),
            item: "q1:shells".to_string(),
            default_resource: None,
            count: PickupCount::Default,
            dropped: false,
            time: SourceTime::Seconds(1.0),
            cargo: None,
            grant: None,
        }
    }

    #[test]
    fn write_keys_match_donor() {
        assert_eq!(
            pickup_write_key(&PickupWrite::Protection {
                channel: ProtectionChannel::Regular
            }),
            "protection:regular"
        );
        assert_eq!(
            pickup_write_key(&PickupWrite::Inventory {
                item: "q1:shells".to_string(),
                fields: InventoryWriteFields::Count,
            }),
            "inventory:q1:shells"
        );
    }

    #[test]
    fn capture_rejects_bad_rules() {
        let good: Rc<dyn OriginalPickupRule> = Rc::new(StubRule {
            id: "r1".to_string(),
            offered: vec!["q1:shells".to_string()],
            writes: vec![PickupWrite::Inventory {
                item: "q1:shells".to_string(),
                fields: InventoryWriteFields::Count,
            }],
            decision: OriginalPickupDecision::Accepted,
        });
        assert_eq!(capture_original_pickup_rules(vec![Rc::clone(&good)]).unwrap().len(), 1);
        let duplicate: Rc<dyn OriginalPickupRule> = Rc::new(StubRule {
            id: "r1".to_string(),
            offered: vec!["q1:nails".to_string()],
            writes: vec![PickupWrite::Inventory {
                item: "q1:nails".to_string(),
                fields: InventoryWriteFields::Count,
            }],
            decision: OriginalPickupDecision::Accepted,
        });
        assert!(capture_original_pickup_rules(vec![good, duplicate]).is_err());
        let empty: Rc<dyn OriginalPickupRule> = Rc::new(StubRule {
            id: "r2".to_string(),
            offered: vec!["q1:cells".to_string()],
            writes: Vec::new(),
            decision: OriginalPickupDecision::Accepted,
        });
        assert!(capture_original_pickup_rules(vec![empty]).is_err());
    }

    #[test]
    fn touch_runs_original_and_blocked_paths() {
        let actors = StubActors::new();
        let recipient = actors.spawn(&provider(), "q1:player");
        let pickup = actors.spawn(&provider(), "q1:shells");
        let admission = SharedOriginalPickupAdmission::new(
            &actors,
            StubCombat {
                matches: StdRefCell::new(Vec::new()),
                stored: StdRefCell::new(Vec::new()),
            },
            StubInventory {
                matches: Vec::new(),
                blocks_primary: false,
            },
            None,
        );
        let continuation = StubContinuation {
            eligible: true,
            original: true,
            completed: StdRefCell::new(Vec::new()),
        };
        assert_eq!(
            admission
                .touch(offer(recipient.id(), pickup.id()), &continuation)
                .unwrap(),
            OriginalPickupOutcome::Accepted
        );
        assert_eq!(*continuation.completed.borrow(), vec![true]);
        admission.assert_idle().unwrap();

        let blocked = SharedOriginalPickupAdmission::new(
            &actors,
            StubCombat {
                matches: StdRefCell::new(Vec::new()),
                stored: StdRefCell::new(Vec::new()),
            },
            StubInventory {
                matches: Vec::new(),
                blocks_primary: true,
            },
            None,
        );
        let continuation = StubContinuation {
            eligible: true,
            original: true,
            completed: StdRefCell::new(Vec::new()),
        };
        assert_eq!(
            blocked
                .touch(offer(recipient.id(), pickup.id()), &continuation)
                .unwrap(),
            OriginalPickupOutcome::Refused
        );
    }

    #[test]
    fn stale_and_ineligible_touches_settle() {
        let actors = StubActors::new();
        let recipient = actors.spawn(&provider(), "q1:player");
        let pickup = actors.spawn(&provider(), "q1:shells");
        actors.retire(&pickup);
        let admission = SharedOriginalPickupAdmission::new(
            &actors,
            StubCombat {
                matches: StdRefCell::new(Vec::new()),
                stored: StdRefCell::new(Vec::new()),
            },
            StubInventory {
                matches: Vec::new(),
                blocks_primary: false,
            },
            None,
        );
        let continuation = StubContinuation {
            eligible: true,
            original: true,
            completed: StdRefCell::new(Vec::new()),
        };
        assert_eq!(
            admission
                .touch(offer(recipient.id(), pickup.id()), &continuation)
                .unwrap(),
            OriginalPickupOutcome::Stale
        );

        let live = actors.spawn(&provider(), "q1:shells");
        let gated = SharedOriginalPickupAdmission::new(
            &actors,
            StubCombat {
                matches: StdRefCell::new(Vec::new()),
                stored: StdRefCell::new(Vec::new()),
            },
            StubInventory {
                matches: Vec::new(),
                blocks_primary: false,
            },
            Some(Box::new(|_| false)),
        );
        assert_eq!(
            gated.touch(offer(recipient.id(), live.id()), &continuation).unwrap(),
            OriginalPickupOutcome::Refused
        );
    }

    #[test]
    fn replacement_grant_consumes_once() {
        let actors = StubActors::new();
        let recipient = actors.spawn(&provider(), "q1:player");
        let pickup = actors.spawn(&provider(), "q1:armor");
        let rule: Rc<dyn OriginalPickupRule> = Rc::new(StubRule {
            id: "armor".to_string(),
            offered: vec!["q1:armor".to_string()],
            writes: vec![PickupWrite::Protection {
                channel: ProtectionChannel::Regular,
            }],
            decision: OriginalPickupDecision::Accepted,
        });
        let captured = Rc::new(
            capture_original_pickup_rules(vec![Rc::clone(&rule)])
                .unwrap()
                .pop()
                .unwrap(),
        );
        let combat = StubCombat {
            matches: StdRefCell::new(Vec::new()),
            stored: StdRefCell::new(Vec::new()),
        };
        combat.matches.borrow_mut().push(CurrentOriginalPickup {
            owner: provider(),
            operation: Rc::clone(&rule),
            captured,
            write: PickupWrite::Protection {
                channel: ProtectionChannel::Regular,
            },
            current: Rc::new(|| true),
        });
        let admission = SharedOriginalPickupAdmission::new(
            &actors,
            combat,
            StubInventory {
                matches: Vec::new(),
                blocks_primary: false,
            },
            None,
        );
        let mut touched = offer(recipient.id(), pickup.id());
        touched.item = "q1:armor".to_string();
        let continuation = StubContinuation {
            eligible: true,
            original: false,
            completed: StdRefCell::new(Vec::new()),
        };
        assert_eq!(
            admission.touch(touched, &continuation).unwrap(),
            OriginalPickupOutcome::Accepted
        );
        assert_eq!(*continuation.completed.borrow(), vec![true]);
    }

    #[test]
    fn run_source_holds_lifetime_for_consumption() {
        let actors = StubActors::new();
        let recipient = actors.spawn(&provider(), "q1:player");
        let pickup = actors.spawn(&provider(), "q1:shells");
        let admission = SharedOriginalPickupAdmission::new(
            &actors,
            StubCombat {
                matches: StdRefCell::new(Vec::new()),
                stored: StdRefCell::new(Vec::new()),
            },
            StubInventory {
                matches: Vec::new(),
                blocks_primary: false,
            },
            None,
        );
        let retired = StdRefCell::new(false);
        admission
            .run_source(offer(recipient.id(), pickup.id()), &|selection, lifetime| {
                assert!(matches!(selection, SourcePickupSelection::Original));
                assert!(admission.assert_idle().is_err());
                lifetime
                    .consume_pickup(&|| {
                        actors.retire(&pickup);
                        retired.replace(true);
                    })
                    .unwrap();
            })
            .unwrap();
        assert!(*retired.borrow());
        admission.assert_idle().unwrap();
    }
}
