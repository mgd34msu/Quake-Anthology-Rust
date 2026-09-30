//! QVM mod pickups: original source pickup rules per recipient.
//!
//! Ports `src/compat/qvm/mod-pickups.ts`. Source calls and actor records
//! come from [`super::mod_actors`]; item storage declarations reuse
//! [`super::item_storage`]; client frames reuse [`super::mod_input`]. The
//! offer/execution/decision mirrors cover the subset of
//! `src/contracts/original-pickups.ts` this module touches; the full rule
//! registry stays host-side.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_core::identity::{ActorId, OwnedActor, ProviderId};

use super::item_storage::{QvmItemCapacity, QvmItemStorage};
use super::mod_actors::{
    QvmCallbackInput, QvmModActorField, QvmModActorRecord, QvmModFieldBinding, QvmModInputs, QvmModReturn,
    QvmModSourceCall, QvmRuntimeValue,
};
use super::mod_input::QvmModTime;
use crate::error::GuestError;

/// Pickup inventory write fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmPickupWriteFields {
    /// Count only.
    Count,
    /// Capacity only.
    Capacity,
    /// Count and capacity.
    CountAndCapacity,
}

/// Protection channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmProtectionChannel {
    /// Regular armor.
    Regular,
    /// Powered armor.
    Powered,
}

/// Declared pickup write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QvmPickupWriteDecl {
    /// Inventory write.
    Inventory {
        /// Item id.
        item: String,
        /// Written fields.
        fields: QvmPickupWriteFields,
    },
    /// Protection write.
    Protection {
        /// Channel.
        channel: QvmProtectionChannel,
    },
}

/// Grant acceptance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmGrantAccepts {
    /// Any nonzero grant result.
    Nonzero,
    /// Always accepts.
    Always,
}

/// Declared pickup operation.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmPickupOperation {
    /// Boolean grant.
    BooleanGrant {
        /// Grant call.
        grant: QvmModSourceCall,
    },
    /// Gate then grant.
    GateThenGrant {
        /// Gate call.
        gate: QvmModSourceCall,
        /// Grant call.
        grant: QvmModSourceCall,
        /// Grant acceptance.
        grant_accepts: QvmGrantAccepts,
    },
}

/// Actor-record context field (mirror of `QvmActorContext`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmActorContext {
    /// Record id.
    pub record: String,
    /// Field offset.
    pub offset: usize,
}

/// Declared source pickup rule.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModPickup {
    /// Rule id.
    pub id: String,
    /// Offered items.
    pub offered: Vec<String>,
    /// Writes (non-empty).
    pub writes: Vec<QvmPickupWriteDecl>,
    /// Operation.
    pub operation: QvmPickupOperation,
    /// Source context fields.
    pub context: Vec<QvmActorContext>,
}

/// Declared protection owner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QvmModPickupProtection {
    /// Channel.
    pub channel: QvmProtectionChannel,
    /// Maximum.
    pub maximum: f64,
}

/// Source-actor lifetime subset for context validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmPickupSourceActors {
    /// In-use flag offset.
    pub inuse: usize,
    /// Reaction field offsets.
    pub callbacks: Vec<usize>,
}

/// Validation context: the declaration subset pickup rules need.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModPickupContext {
    /// Protection owners.
    pub protection: Vec<QvmModPickupProtection>,
    /// Actor records.
    pub actor_records: Vec<QvmModActorRecord>,
    /// Engine entity record id.
    pub entity_record: Option<String>,
    /// Source-actor lifetime.
    pub source_actors: Option<QvmPickupSourceActors>,
    /// Client record ids.
    pub clients_records: Vec<String>,
    /// Item storage declarations.
    pub items_storage: Option<Vec<QvmItemStorage>>,
}

/// Offered pickup count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmPickupCount {
    /// Default count.
    Default,
    /// Override amount.
    Override {
        /// Amount.
        amount: i32,
    },
}

/// Original pickup offer (used subset).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmOriginalPickupOffer {
    /// Recipient.
    pub recipient: ActorId,
    /// Pickup.
    pub pickup: ActorId,
    /// Item id.
    pub item: String,
    /// Time.
    pub time: QvmModTime,
    /// Count.
    pub count: QvmPickupCount,
    /// Whether the pickup was dropped.
    pub dropped: bool,
}

/// Original pickup execution (used subset).
#[derive(Clone)]
pub struct QvmOriginalPickupExecution {
    /// Currency check.
    pub current: Rc<dyn Fn() -> bool>,
}

/// Original pickup decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmPickupDecision {
    /// Accepted.
    Accepted,
    /// Refused.
    Refused,
}

/// Bound source pickup rule.
#[derive(Clone)]
pub struct QvmOriginalPickupRule {
    /// Rule id.
    pub id: String,
    /// Offered items.
    pub offered: Vec<String>,
    /// Writes.
    pub writes: Vec<QvmPickupWriteDecl>,
    /// Take handler.
    pub take: Rc<dyn Fn(&QvmOriginalPickupOffer, &QvmOriginalPickupExecution) -> Result<QvmPickupDecision, GuestError>>,
}

/// Inventory pickup binding.
#[derive(Clone)]
pub struct QvmPickupInventoryBinding {
    /// Owning provider.
    pub owner: ProviderId,
    /// Rules.
    pub rules: Vec<QvmOriginalPickupRule>,
}

/// Host services for pickup rules.
pub trait QvmModPickupServices {
    /// Current destination clients.
    fn clients(&self) -> Vec<ActorId>;
    /// Resolve an owned actor.
    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor>;
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Bind pickup rules; returns the remover.
    fn bind_pickup(
        &self,
        owned: &OwnedActor,
        binding: QvmPickupInventoryBinding,
    ) -> Result<Box<dyn FnOnce()>, GuestError>;
}

/// Provider operations for pickup rules.
pub trait QvmModPickupOperations {
    /// Refresh currency.
    fn current(&self);
    /// Whether a recipient is eligible.
    fn eligible(&self, actor: &ActorId) -> bool;
    /// Run source context around observation.
    fn context(
        &self,
        definition: &QvmModPickup,
        offer: &QvmOriginalPickupOffer,
        inputs: &QvmModInputs,
        observe: &dyn Fn() -> Result<QvmPickupDecision, GuestError>,
    ) -> Result<QvmPickupDecision, GuestError>;
    /// Observe a recipient around a grant.
    fn observe(
        &self,
        actor: &ActorId,
        current: &dyn Fn() -> bool,
        proceed: &dyn Fn() -> Result<QvmPickupDecision, GuestError>,
    ) -> Result<QvmPickupDecision, GuestError>;
    /// Invoke a source call.
    fn invoke(&self, call: &QvmModSourceCall, inputs: &QvmModInputs) -> Result<i32, GuestError>;
}

/// Field storage width.
fn field_width(field: &QvmModActorField) -> usize {
    match &field.binding {
        QvmModFieldBinding::Private { byte_length } => *byte_length,
        QvmModFieldBinding::Origin
        | QvmModFieldBinding::Velocity
        | QvmModFieldBinding::Angles
        | QvmModFieldBinding::BoundsMin
        | QvmModFieldBinding::BoundsMax
        | QvmModFieldBinding::ConstantVector(_) => 12,
        _ => 4,
    }
}

/// Validate pickup rules against their declaration context.
pub fn validate_qvm_mod_pickups(rules: &[QvmModPickup], context: &QvmModPickupContext) -> Result<(), GuestError> {
    for rule in rules {
        if rule.id.is_empty() || rule.offered.is_empty() || rule.writes.is_empty() {
            return Err(GuestError::invalid("Invalid QVM pickup source rule"));
        }
        for write in &rule.writes {
            match write {
                QvmPickupWriteDecl::Protection { channel } => {
                    if !context.protection.iter().any(|owner| owner.channel == *channel) {
                        return Err(GuestError::invalid("QVM pickup has no protection owner"));
                    }
                }
                QvmPickupWriteDecl::Inventory { item, fields } => {
                    let projected = *fields == QvmPickupWriteFields::Count
                        && context.actor_records.iter().any(|record| {
                            record.fields.iter().any(|field| {
                                matches!(
                                    &field.binding,
                                    QvmModFieldBinding::Inventory {
                                        item: bound, ..
                                    } if bound == item
                                )
                            })
                        });
                    let owned = context
                        .items_storage
                        .as_deref()
                        .unwrap_or(&[])
                        .iter()
                        .any(|storage| match storage {
                            QvmItemStorage::Bits { items, .. } => {
                                *fields == QvmPickupWriteFields::Count && items.iter().any(|entry| entry.item == *item)
                            }
                            QvmItemStorage::Counter {
                                item: owned, capacity, ..
                            } => {
                                owned == item
                                    && (*fields == QvmPickupWriteFields::Count
                                        || matches!(capacity, QvmItemCapacity::Field { .. }))
                            }
                        });
                    if !projected && !owned {
                        return Err(GuestError::invalid(
                            "QVM pickup has no declared inventory storage for its requested count/capacity writes",
                        ));
                    }
                }
            }
        }
        let decisions = match &rule.operation {
            QvmPickupOperation::BooleanGrant { grant } => grant.returns == QvmModReturn::Void,
            QvmPickupOperation::GateThenGrant {
                gate,
                grant,
                grant_accepts,
            } => {
                gate.returns == QvmModReturn::Void
                    || *grant_accepts == QvmGrantAccepts::Nonzero && grant.returns == QvmModReturn::Void
            }
        };
        if decisions {
            return Err(GuestError::invalid("QVM pickup requires its declared source decision"));
        }
        let mut occupied = HashSet::new();
        for field in &rule.context {
            let record = context.actor_records.iter().find(|record| record.id == field.record);
            let key = format!("{}:{}", field.record, field.offset);
            let Some(record) = record else {
                return Err(GuestError::invalid("Invalid QVM pickup source context"));
            };
            if context.clients_records.contains(&record.id)
                || field.offset % 4 != 0
                || field.offset + 4 > record.stride
                || !occupied.insert(key)
            {
                return Err(GuestError::invalid("Invalid QVM pickup source context"));
            }
            for binding in &record.fields {
                let width = field_width(binding);
                let shared = !matches!(
                    binding.binding,
                    QvmModFieldBinding::Private { .. } | QvmModFieldBinding::Constant { .. }
                );
                if binding.offset < field.offset + 4 && field.offset < binding.offset + width && shared {
                    return Err(GuestError::invalid(
                        "QVM pickup context overlaps shared or linked storage",
                    ));
                }
            }
            if Some(record.id.as_str()) == context.entity_record.as_deref() {
                if let Some(source) = context.source_actors.as_ref() {
                    if field.offset == source.inuse || source.callbacks.contains(&field.offset) {
                        return Err(GuestError::invalid("QVM pickup context overlaps source actor lifetime"));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Rules travel with the current resource binding; this object owns only source execution and inventory delegates.
pub struct QvmModPickups {
    /// Rule definitions.
    definitions: Vec<QvmModPickup>,
    /// Host services.
    services: Rc<dyn QvmModPickupServices>,
    /// Owning provider.
    owner: ProviderId,
    /// Provider operations.
    operations: Rc<dyn QvmModPickupOperations>,
    /// Whether delegates are bound.
    active: Cell<bool>,
    /// Per-actor rules.
    rules: RefCell<HashMap<ActorId, Vec<QvmOriginalPickupRule>>>,
    /// Execution depth.
    depth: Cell<usize>,
    /// Per-actor delegate removers.
    delegates: RefCell<HashMap<ActorId, Vec<Box<dyn FnOnce()>>>>,
}

impl QvmModPickups {
    /// Bind pickup definitions.
    pub fn create(
        definitions: Vec<QvmModPickup>,
        services: Rc<dyn QvmModPickupServices>,
        owner: ProviderId,
        operations: Rc<dyn QvmModPickupOperations>,
    ) -> Rc<Self> {
        Rc::new(Self {
            definitions,
            services,
            owner,
            operations,
            active: Cell::new(false),
            rules: RefCell::new(HashMap::new()),
            depth: Cell::new(0),
            delegates: RefCell::new(HashMap::new()),
        })
    }

    /// Whether delegates are bound.
    pub fn is_active(&self) -> bool {
        self.active.get()
    }

    /// Rules writing a protection channel.
    pub fn protection(self: &Rc<Self>, actor: &ActorId, channel: QvmProtectionChannel) -> Vec<QvmOriginalPickupRule> {
        self.actor_rules(actor)
            .into_iter()
            .filter(|rule| {
                rule.writes.iter().any(|write| {
                    matches!(
                        write,
                        QvmPickupWriteDecl::Protection {
                            channel: bound,
                        } if *bound == channel
                    )
                })
            })
            .collect()
    }

    /// Memoized per-actor rules.
    fn actor_rules(self: &Rc<Self>, actor: &ActorId) -> Vec<QvmOriginalPickupRule> {
        if let Some(rules) = self.rules.borrow().get(actor) {
            return rules.clone();
        }
        let rules = self
            .definitions
            .iter()
            .map(|definition| {
                let this = Rc::clone(self);
                let actor = actor.clone();
                let definition = definition.clone();
                QvmOriginalPickupRule {
                    id: definition.id.clone(),
                    offered: definition.offered.clone(),
                    writes: definition.writes.clone(),
                    take: Rc::new(move |offer, execution| this.take(&actor, &definition, offer, execution)),
                }
            })
            .collect::<Vec<_>>();
        self.rules.borrow_mut().insert(actor.clone(), rules.clone());
        rules
    }

    /// Activate delegates for current clients.
    pub fn activate(self: &Rc<Self>) -> Result<(), GuestError> {
        self.operations.current();
        self.active.set(true);
        for actor in self.services.clients() {
            self.bind_actor(&actor)?;
        }
        Ok(())
    }

    /// Bind one recipient.
    pub fn bind_actor(self: &Rc<Self>, actor: &ActorId) -> Result<(), GuestError> {
        if !self.active.get() || !self.operations.eligible(actor) || self.delegates.borrow().contains_key(actor) {
            return Ok(());
        }
        let owned = self
            .services
            .resolve_owned(actor)
            .ok_or_else(|| GuestError::invalid("QVM pickup recipient is retired"))?;
        let rules = self
            .actor_rules(actor)
            .into_iter()
            .filter(|rule| {
                rule.writes
                    .iter()
                    .any(|write| matches!(write, QvmPickupWriteDecl::Inventory { .. }))
            })
            .collect::<Vec<_>>();
        let mut removers = Vec::new();
        if !rules.is_empty() {
            match self.services.bind_pickup(
                &owned,
                QvmPickupInventoryBinding {
                    owner: self.owner.clone(),
                    rules,
                },
            ) {
                Ok(remove) => removers.push(remove),
                Err(error) => {
                    while let Some(remove) = removers.pop() {
                        remove();
                    }
                    return Err(error);
                }
            }
        }
        self.delegates.borrow_mut().insert(actor.clone(), removers);
        Ok(())
    }

    /// Take an offered pickup through source execution.
    fn take(
        &self,
        actor: &ActorId,
        definition: &QvmModPickup,
        offer: &QvmOriginalPickupOffer,
        execution: &QvmOriginalPickupExecution,
    ) -> Result<QvmPickupDecision, GuestError> {
        self.operations.current();
        let current = || {
            (execution.current)()
                && self.active.get()
                && self.services.is_live(actor)
                && self.operations.eligible(actor)
                && self.services.is_live(&offer.pickup)
        };
        if offer.recipient != *actor || !definition.offered.contains(&offer.item) || !current() {
            return Err(GuestError::invalid("QVM pickup rule is no longer current"));
        }
        let mut inputs = QvmModInputs::new();
        inputs.insert(
            QvmCallbackInput::named("self"),
            QvmRuntimeValue::Actor(Some(actor.clone())),
        );
        inputs.insert(
            QvmCallbackInput::named("other"),
            QvmRuntimeValue::Actor(Some(offer.pickup.clone())),
        );
        inputs.insert(
            QvmCallbackInput::named("item"),
            QvmRuntimeValue::Text(offer.item.clone()),
        );
        inputs.insert(
            QvmCallbackInput::named("time"),
            QvmRuntimeValue::Float(match offer.time {
                QvmModTime::Seconds(value) => value,
                QvmModTime::Milliseconds(value) => f64::from(value) / 1000.0,
            }),
        );
        inputs.insert(
            QvmCallbackInput::named("pickup-count"),
            QvmRuntimeValue::Float(match offer.count {
                QvmPickupCount::Override { amount } => f64::from(amount),
                QvmPickupCount::Default => 0.0,
            }),
        );
        inputs.insert(
            QvmCallbackInput::named("pickup-has-count"),
            QvmRuntimeValue::Float(match offer.count {
                QvmPickupCount::Override { .. } => 1.0,
                QvmPickupCount::Default => 0.0,
            }),
        );
        inputs.insert(
            QvmCallbackInput::named("pickup-dropped"),
            QvmRuntimeValue::Float(if offer.dropped { 1.0 } else { 0.0 }),
        );
        self.depth.set(self.depth.get() + 1);
        let decision = self.operations.context(definition, offer, &inputs, &|| {
            self.operations.observe(actor, &current, &|| {
                let operation = &definition.operation;
                if let QvmPickupOperation::GateThenGrant { gate, .. } = operation {
                    if self.operations.invoke(gate, &inputs)? == 0 || !current() {
                        return Ok(QvmPickupDecision::Refused);
                    }
                }
                let grant = match operation {
                    QvmPickupOperation::BooleanGrant { grant } => grant,
                    QvmPickupOperation::GateThenGrant { grant, .. } => grant,
                };
                let result = self.operations.invoke(grant, &inputs)?;
                if matches!(
                    operation,
                    QvmPickupOperation::GateThenGrant {
                        grant_accepts: QvmGrantAccepts::Always,
                        ..
                    }
                ) || result != 0
                {
                    Ok(QvmPickupDecision::Accepted)
                } else {
                    Ok(QvmPickupDecision::Refused)
                }
            })
        });
        self.depth.set(self.depth.get() - 1);
        decision
    }

    /// Fail while pickup execution is active.
    pub fn assert_idle(&self) -> Result<(), GuestError> {
        if self.depth.get() != 0 {
            return Err(GuestError::invalid(
                "Cannot save or restore during QVM original pickup execution",
            ));
        }
        Ok(())
    }

    /// Release one recipient.
    pub fn release(&self, actor: &ActorId) {
        let removers = self.delegates.borrow_mut().remove(actor);
        self.rules.borrow_mut().remove(actor);
        for remove in removers.unwrap_or_default() {
            remove();
        }
    }

    /// Release all recipients.
    pub fn close(&self) {
        self.active.set(false);
        self.rules.borrow_mut().clear();
        let actors: Vec<ActorId> = self.delegates.borrow().keys().cloned().collect();
        for actor in actors {
            self.release(&actor);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use qa_core::identity::{IdentityOwner, ProviderId};

    use super::super::item_storage::QvmItemField;
    use super::super::mod_actors::{QvmModActorField, QvmModFieldBinding, QvmModScalar};
    use super::*;

    struct FixtureServices {
        clients: Vec<ActorId>,
        owned: RefCell<std::collections::HashMap<ActorId, OwnedActor>>,
        live: Rc<Cell<bool>>,
        bound: RefCell<Vec<QvmPickupInventoryBinding>>,
        removed: Rc<Cell<usize>>,
    }

    impl QvmModPickupServices for FixtureServices {
        fn clients(&self) -> Vec<ActorId> {
            self.clients.clone()
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            self.owned.borrow().get(actor).cloned()
        }

        fn is_live(&self, _actor: &ActorId) -> bool {
            self.live.get()
        }

        fn bind_pickup(
            &self,
            _owned: &OwnedActor,
            binding: QvmPickupInventoryBinding,
        ) -> Result<Box<dyn FnOnce()>, GuestError> {
            self.bound.borrow_mut().push(binding);
            let removed = Rc::clone(&self.removed);
            Ok(Box::new(move || removed.set(removed.get() + 1)))
        }
    }

    struct FixtureOperations {
        eligible: Rc<Cell<bool>>,
        invoked: RefCell<Vec<(usize, QvmModInputs)>>,
        gate: Cell<i32>,
        grant: Cell<i32>,
    }

    impl QvmModPickupOperations for FixtureOperations {
        fn current(&self) {}

        fn eligible(&self, _actor: &ActorId) -> bool {
            self.eligible.get()
        }

        fn context(
            &self,
            _definition: &QvmModPickup,
            _offer: &QvmOriginalPickupOffer,
            _inputs: &QvmModInputs,
            observe: &dyn Fn() -> Result<QvmPickupDecision, GuestError>,
        ) -> Result<QvmPickupDecision, GuestError> {
            observe()
        }

        fn observe(
            &self,
            _actor: &ActorId,
            current: &dyn Fn() -> bool,
            proceed: &dyn Fn() -> Result<QvmPickupDecision, GuestError>,
        ) -> Result<QvmPickupDecision, GuestError> {
            if current() {
                proceed()
            } else {
                Ok(QvmPickupDecision::Refused)
            }
        }

        fn invoke(&self, call: &QvmModSourceCall, inputs: &QvmModInputs) -> Result<i32, GuestError> {
            self.invoked.borrow_mut().push((call.entry, inputs.clone()));
            if call.entry == 1 {
                Ok(self.gate.get())
            } else {
                Ok(self.grant.get())
            }
        }
    }

    struct Fixture {
        pickups: Rc<QvmModPickups>,
        services: Rc<FixtureServices>,
        operations: Rc<FixtureOperations>,
        recipient: ActorId,
        pickup: ActorId,
    }

    fn source_call(entry: usize, returns: QvmModReturn) -> QvmModSourceCall {
        QvmModSourceCall {
            entry,
            arguments: Vec::new(),
            globals: Vec::new(),
            returns,
        }
    }

    fn rule() -> QvmModPickup {
        QvmModPickup {
            id: "shells".to_string(),
            offered: vec!["shells".to_string()],
            writes: vec![QvmPickupWriteDecl::Inventory {
                item: "shells".to_string(),
                fields: QvmPickupWriteFields::Count,
            }],
            operation: QvmPickupOperation::GateThenGrant {
                gate: source_call(1, QvmModReturn::Int32),
                grant: source_call(2, QvmModReturn::Int32),
                grant_accepts: QvmGrantAccepts::Nonzero,
            },
            context: Vec::new(),
        }
    }

    fn context() -> QvmModPickupContext {
        QvmModPickupContext {
            protection: vec![QvmModPickupProtection {
                channel: QvmProtectionChannel::Regular,
                maximum: 100.0,
            }],
            actor_records: vec![QvmModActorRecord {
                id: "entity".to_string(),
                address: 4096,
                stride: 512,
                capacity: 4,
                fields: vec![QvmModActorField {
                    offset: 64,
                    access: None,
                    binding: QvmModFieldBinding::Inventory {
                        encoding: QvmModScalar::Int32,
                        item: "shells".to_string(),
                    },
                }],
            }],
            entity_record: Some("entity".to_string()),
            source_actors: Some(QvmPickupSourceActors {
                inuse: 208,
                callbacks: vec![248],
            }),
            clients_records: vec!["client".to_string()],
            items_storage: Some(vec![QvmItemStorage::Counter {
                field: QvmItemField {
                    record: "entity".to_string(),
                    offset: 64,
                },
                item: "shells".to_string(),
                capacity: QvmItemCapacity::Constant { value: 50 },
            }]),
        }
    }

    fn fixture() -> Fixture {
        let owner = IdentityOwner::create("mod-pickups-test").unwrap();
        let recipient = owner.actor(0, 1);
        let pickup = owner.actor(1, 1);
        let owned = owner.owned_actor(&recipient, ProviderId::new("test", "mod")).unwrap();
        let services = Rc::new(FixtureServices {
            clients: vec![recipient.clone()],
            owned: RefCell::new([(recipient.clone(), owned)].into_iter().collect()),
            live: Rc::new(Cell::new(true)),
            bound: RefCell::new(Vec::new()),
            removed: Rc::new(Cell::new(0)),
        });
        let operations = Rc::new(FixtureOperations {
            eligible: Rc::new(Cell::new(true)),
            invoked: RefCell::new(Vec::new()),
            gate: Cell::new(1),
            grant: Cell::new(1),
        });
        let pickups = QvmModPickups::create(
            vec![rule()],
            Rc::clone(&services) as Rc<dyn QvmModPickupServices>,
            ProviderId::new("test", "mod"),
            Rc::clone(&operations) as Rc<dyn QvmModPickupOperations>,
        );
        Fixture {
            pickups,
            services,
            operations,
            recipient,
            pickup,
        }
    }

    fn offer(fixture: &Fixture) -> (QvmOriginalPickupOffer, QvmOriginalPickupExecution) {
        (
            QvmOriginalPickupOffer {
                recipient: fixture.recipient.clone(),
                pickup: fixture.pickup.clone(),
                item: "shells".to_string(),
                time: QvmModTime::Milliseconds(1500),
                count: QvmPickupCount::Override { amount: 8 },
                dropped: true,
            },
            QvmOriginalPickupExecution {
                current: Rc::new(|| true),
            },
        )
    }

    #[test]
    fn validates_rules() {
        assert!(validate_qvm_mod_pickups(&[rule()], &context()).is_ok());

        let mut bad = rule();
        bad.id.clear();
        assert!(validate_qvm_mod_pickups(&[bad], &context()).is_err());

        let mut bad = rule();
        bad.writes = vec![QvmPickupWriteDecl::Protection {
            channel: QvmProtectionChannel::Powered,
        }];
        assert!(validate_qvm_mod_pickups(&[bad], &context()).is_err());

        let mut bad = rule();
        bad.writes = vec![QvmPickupWriteDecl::Inventory {
            item: "nails".to_string(),
            fields: QvmPickupWriteFields::Count,
        }];
        assert!(validate_qvm_mod_pickups(&[bad], &context()).is_err());

        let mut bad = rule();
        bad.operation = QvmPickupOperation::BooleanGrant {
            grant: source_call(2, QvmModReturn::Void),
        };
        assert!(validate_qvm_mod_pickups(&[bad], &context()).is_err());

        let mut bad = rule();
        bad.operation = QvmPickupOperation::GateThenGrant {
            gate: source_call(1, QvmModReturn::Void),
            grant: source_call(2, QvmModReturn::Int32),
            grant_accepts: QvmGrantAccepts::Nonzero,
        };
        assert!(validate_qvm_mod_pickups(&[bad], &context()).is_err());

        let mut bad = rule();
        bad.context = vec![QvmActorContext {
            record: "entity".to_string(),
            offset: 64,
        }];
        assert!(validate_qvm_mod_pickups(&[bad], &context()).is_err());

        let mut bad = rule();
        bad.context = vec![QvmActorContext {
            record: "client".to_string(),
            offset: 0,
        }];
        assert!(validate_qvm_mod_pickups(&[bad], &context()).is_err());

        let mut bad = rule();
        bad.context = vec![QvmActorContext {
            record: "entity".to_string(),
            offset: 208,
        }];
        assert!(validate_qvm_mod_pickups(&[bad], &context()).is_err());

        let mut good = rule();
        good.context = vec![QvmActorContext {
            record: "entity".to_string(),
            offset: 100,
        }];
        assert!(validate_qvm_mod_pickups(&[good], &context()).is_ok());
    }

    #[test]
    fn activate_binds_and_takes() {
        let fixture = fixture();
        assert!(!fixture.pickups.is_active());
        fixture.pickups.activate().unwrap();
        assert!(fixture.pickups.is_active());
        assert_eq!(fixture.services.bound.borrow().len(), 1);

        let (offer, execution) = offer(&fixture);
        let rules = fixture.services.bound.borrow()[0].rules.clone();
        assert_eq!(rules.len(), 1);
        let decision = (rules[0].take)(&offer, &execution).unwrap();
        assert_eq!(decision, QvmPickupDecision::Accepted);
        {
            let invoked = fixture.operations.invoked.borrow();
            assert_eq!(invoked.len(), 2);
            assert_eq!(invoked[0].0, 1);
            assert_eq!(invoked[1].0, 2);
            let inputs = &invoked[1].1;
            assert_eq!(
                inputs.get(&QvmCallbackInput::named("item")),
                Some(&QvmRuntimeValue::Text("shells".to_string()))
            );
            assert_eq!(
                inputs.get(&QvmCallbackInput::named("time")),
                Some(&QvmRuntimeValue::Float(1.5))
            );
            assert_eq!(
                inputs.get(&QvmCallbackInput::named("pickup-count")),
                Some(&QvmRuntimeValue::Float(8.0))
            );
            assert_eq!(
                inputs.get(&QvmCallbackInput::named("pickup-dropped")),
                Some(&QvmRuntimeValue::Float(1.0))
            );
        }
        assert!(fixture
            .pickups
            .protection(&fixture.recipient, QvmProtectionChannel::Regular)
            .is_empty());

        let (mut offer, execution) = offer(&fixture);
        offer.count = QvmPickupCount::Default;
        let decision = (rules[0].take)(&offer, &execution).unwrap();
        assert_eq!(decision, QvmPickupDecision::Accepted);
        let invoked = fixture.operations.invoked.borrow();
        assert_eq!(
            invoked.last().unwrap().1.get(&QvmCallbackInput::named("pickup-count")),
            Some(&QvmRuntimeValue::Float(0.0))
        );
    }

    #[test]
    fn gates_and_grants_decide() {
        let fixture = fixture();
        fixture.pickups.activate().unwrap();
        let (offer, execution) = offer(&fixture);
        let rules = fixture.services.bound.borrow()[0].rules.clone();

        fixture.operations.gate.set(0);
        assert_eq!((rules[0].take)(&offer, &execution).unwrap(), QvmPickupDecision::Refused);
        fixture.operations.gate.set(1);
        fixture.operations.grant.set(0);
        assert_eq!((rules[0].take)(&offer, &execution).unwrap(), QvmPickupDecision::Refused);
        assert!(fixture.pickups.assert_idle().is_ok());
    }

    #[test]
    fn stale_takes_fail() {
        let fixture = fixture();
        fixture.pickups.activate().unwrap();
        let (mut offer, execution) = offer(&fixture);
        let rules = fixture.services.bound.borrow()[0].rules.clone();
        offer.item = "nails".to_string();
        assert!((rules[0].take)(&offer, &execution).is_err());

        let (offer, _) = offer(&fixture);
        let dead = QvmOriginalPickupExecution {
            current: Rc::new(|| false),
        };
        assert!((rules[0].take)(&offer, &dead).is_err());

        fixture.services.live.set(false);
        let (offer, execution) = offer(&fixture);
        assert!((rules[0].take)(&offer, &execution).is_err());
    }

    #[test]
    fn release_and_close_cleanup() {
        let fixture = fixture();
        fixture.pickups.activate().unwrap();
        fixture.pickups.release(&fixture.recipient);
        assert_eq!(fixture.services.removed.get(), 1);
        fixture.pickups.activate().unwrap();
        fixture.pickups.close();
        assert!(!fixture.pickups.is_active());
        assert_eq!(fixture.services.removed.get(), 2);
    }
}
