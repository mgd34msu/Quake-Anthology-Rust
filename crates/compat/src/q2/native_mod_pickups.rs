//! Port of `src/compat/q2/native-mod-pickups.ts`.
//! Bridges original pickup decisions: gate and grant calls run in the source
//! while rules travel with the current resource binding.

use std::collections::{HashMap, HashSet};

use qa_core::math::Vec3;
use thiserror::Error;

/// Failures in the native pickup bridge.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum PickupError {
    /// The bridge is not current.
    #[error("native pickup bridge is not current: {0}")]
    NotCurrent(String),
    /// A pickup rule is no longer current.
    #[error("native pickup rule is no longer current")]
    StaleRule,
    /// A pickup recipient is retired.
    #[error("native pickup recipient is retired")]
    RecipientRetired,
    /// Pickup rules need unique ids, offers, and client admission.
    #[error("native pickups require unique rules and source client admission")]
    BadRules,
    /// Two rules offer the same item.
    #[error("ambiguous native original pickup item")]
    AmbiguousItem,
    /// A pickup names unowned protection.
    #[error("native pickup has no protection owner")]
    MissingProtection,
    /// A pickup names undeclared inventory storage.
    #[error("native pickup has no declared inventory storage")]
    MissingInventory,
    /// A pickup names undeclared capacity storage.
    #[error("native pickup has no declared inventory capacity storage")]
    MissingCapacity,
    /// A pickup lacks its declared source decision.
    #[error("native pickup requires its declared source decision")]
    MissingDecision,
    /// Cleanup aggregated failures.
    #[error("native pickup cleanup failed: {0:?}")]
    Cleanup(Vec<String>),
}

/// Generational actor handle local to the pickup bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeActorId {
    /// Registry slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
}

/// Protection channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtectionChannel {
    /// Regular armor.
    Regular,
    /// Powered protection.
    Powered,
}

/// Inventory write fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickupFields {
    /// Count only.
    Count,
    /// Capacity only.
    Capacity,
    /// Count and capacity.
    Both,
}

/// One declared resource write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickupWrite {
    /// Inventory write.
    Inventory {
        /// Target item.
        item: String,
        /// Written fields.
        fields: PickupFields,
    },
    /// Protection write.
    Protection {
        /// Target channel.
        channel: ProtectionChannel,
    },
}

/// Declared source call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickupCall {
    /// Call id.
    pub id: String,
    /// Whether the call returns its decision value.
    pub returns_value: bool,
}

/// Grant acceptance rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantAccepts {
    /// Any grant result accepts.
    Always,
    /// Zero refuses.
    NonZero,
}

/// Pickup operation shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickupOperation {
    /// Single grant call with a boolean decision.
    BooleanGrant {
        /// Grant call.
        grant: PickupCall,
    },
    /// Gate call followed by a grant call.
    GateThenGrant {
        /// Gate call.
        gate: PickupCall,
        /// Grant call.
        grant: PickupCall,
        /// Grant acceptance.
        grant_accepts: GrantAccepts,
    },
}

/// Pickup rule declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickupDefinition {
    /// Rule id.
    pub id: String,
    /// Offered items.
    pub offered: Vec<String>,
    /// Declared writes.
    pub writes: Vec<PickupWrite>,
    /// Operation shape.
    pub operation: PickupOperation,
}

/// Validate pickup declarations against inventory and protection owners.
pub fn validate_native_mod_pickups(
    definitions: &[PickupDefinition],
    has_clients: bool,
    protection_channels: &[ProtectionChannel],
    inventory_items: &[String],
    capacity_items: &[String],
) -> Result<(), PickupError> {
    let mut ids = HashSet::new();
    let mut offered = HashSet::new();
    for rule in definitions {
        if !has_clients || !ids.insert(rule.id.clone()) || rule.id.is_empty() || rule.offered.is_empty() {
            return Err(PickupError::BadRules);
        }
        for item in &rule.offered {
            if !offered.insert(item.clone()) {
                return Err(PickupError::AmbiguousItem);
            }
        }
        for write in &rule.writes {
            match write {
                PickupWrite::Protection { channel } => {
                    if !protection_channels.contains(channel) {
                        return Err(PickupError::MissingProtection);
                    }
                }
                PickupWrite::Inventory { item, fields } => {
                    if *fields != PickupFields::Capacity && !inventory_items.contains(item) {
                        return Err(PickupError::MissingInventory);
                    }
                    if *fields != PickupFields::Count && !capacity_items.contains(item) {
                        return Err(PickupError::MissingCapacity);
                    }
                }
            }
        }
        match &rule.operation {
            PickupOperation::BooleanGrant { grant } => {
                if !grant.returns_value {
                    return Err(PickupError::MissingDecision);
                }
            }
            PickupOperation::GateThenGrant {
                gate,
                grant,
                grant_accepts,
            } => {
                if !gate.returns_value || (*grant_accepts == GrantAccepts::NonZero && !grant.returns_value) {
                    return Err(PickupError::MissingDecision);
                }
            }
        }
    }
    Ok(())
}

/// Pickup offer across the boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct PickupOffer {
    /// Receiving actor.
    pub recipient: NativeActorId,
    /// Pickup actor.
    pub pickup: NativeActorId,
    /// Offered item.
    pub item: String,
    /// Count override, if any.
    pub count_override: Option<f64>,
    /// Whether the pickup was dropped.
    pub dropped: bool,
    /// Offer time in seconds.
    pub time_secs: f64,
}

/// Pickup decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickupDecision {
    /// The source granted the pickup.
    Accepted,
    /// The source refused the pickup.
    Refused,
}

/// Runtime input value.
#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeValue {
    /// Actor handle.
    Actor(Option<NativeActorId>),
    /// Scalar.
    Float(f64),
    /// String.
    Text(String),
    /// Vector.
    Vector(Vec3),
}

/// Source call inputs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PickupInputs {
    values: HashMap<String, RuntimeValue>,
}

impl PickupInputs {
    /// Read one input.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&RuntimeValue> {
        self.values.get(name)
    }

    /// All input names.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.values.keys().cloned().collect()
    }
}

/// Live resource binding for one pickup execution.
pub trait PickupExecution {
    /// Whether the binding is current.
    fn is_current(&self) -> bool;
    /// Declared writes.
    fn writes(&self) -> &[PickupWrite];
}

/// Pickup operations: context, observation, and source calls.
pub trait PickupOperations {
    /// Assert the bridge is current.
    fn assert_current(&self) -> Result<(), PickupError>;
    /// Whether an actor is live.
    fn is_live(&self, actor: NativeActorId) -> bool;
    /// Whether an actor is an eligible recipient.
    fn is_eligible(&self, actor: NativeActorId) -> bool;
    /// Connected client actors.
    fn client_actors(&self) -> Vec<NativeActorId>;
    /// Run a closure inside pickup source context.
    fn context<R>(
        &mut self,
        definition: &PickupDefinition,
        offer: &PickupOffer,
        inputs: &PickupInputs,
        execute: impl FnOnce(&mut Self) -> R,
    ) -> R;
    /// Run a closure under pickup observation.
    fn observe<R>(
        &mut self,
        actor: NativeActorId,
        execution: &dyn PickupExecution,
        execute: impl FnOnce(&mut Self) -> R,
    ) -> R;
    /// Invoke a source call; `None` means retired.
    fn invoke(&mut self, call: &PickupCall, inputs: &PickupInputs) -> Option<i32>;
    /// Bind inventory rules for an actor; returns a delegate token.
    fn bind_pickup_rules(&mut self, actor: NativeActorId, rule_ids: &[String]) -> u64;
    /// Release a delegate token.
    fn unbind_pickup_rules(&mut self, token: u64) -> Result<(), PickupError>;
}

/// One bound pickup rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickupRule {
    /// Rule id.
    pub id: String,
    /// Offered items.
    pub offered: Vec<String>,
    /// Declared writes.
    pub writes: Vec<PickupWrite>,
    /// Owning recipient.
    pub actor: NativeActorId,
}

fn is_current<O: PickupOperations>(
    operations: &O,
    execution: &dyn PickupExecution,
    active: bool,
    actor: NativeActorId,
    pickup: NativeActorId,
) -> bool {
    execution.is_current()
        && active
        && operations.is_live(actor)
        && operations.is_eligible(actor)
        && operations.is_live(pickup)
}

/// Native pickup bridge over synthetic operations.
pub struct NativeModPickups<O: PickupOperations> {
    definitions: Vec<PickupDefinition>,
    owner: String,
    operations: O,
    active: bool,
    rules: HashMap<NativeActorId, Vec<PickupRule>>,
    delegates: HashMap<NativeActorId, Vec<u64>>,
    depth: u32,
}

impl<O: PickupOperations> NativeModPickups<O> {
    /// Build the bridge.
    pub fn new(definitions: Vec<PickupDefinition>, owner: &str, operations: O) -> Self {
        Self {
            definitions,
            owner: owner.to_string(),
            operations,
            active: false,
            rules: HashMap::new(),
            delegates: HashMap::new(),
            depth: 0,
        }
    }

    /// Owning provider name.
    #[must_use]
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Borrow the operations (for fixtures).
    #[must_use]
    pub fn operations(&self) -> &O {
        &self.operations
    }

    /// Mutably borrow the operations (for fixtures).
    pub fn operations_mut(&mut self) -> &mut O {
        &mut self.operations
    }

    /// Whether a pickup execution is in flight.
    #[must_use]
    pub fn in_progress(&self) -> bool {
        self.depth != 0
    }

    /// Rules writing one protection channel for an actor.
    pub fn protection(&mut self, actor: NativeActorId, channel: ProtectionChannel) -> Vec<PickupRule> {
        self.actor_rules(actor)
            .into_iter()
            .filter(|rule| {
                rule.writes
                    .iter()
                    .any(|write| matches!(write, PickupWrite::Protection { channel: bound } if *bound == channel))
            })
            .collect()
    }

    fn actor_rules(&mut self, actor: NativeActorId) -> Vec<PickupRule> {
        if let Some(rules) = self.rules.get(&actor) {
            return rules.clone();
        }
        let rules: Vec<PickupRule> = self
            .definitions
            .iter()
            .map(|definition| PickupRule {
                id: definition.id.clone(),
                offered: definition.offered.clone(),
                writes: definition.writes.clone(),
                actor,
            })
            .collect();
        self.rules.insert(actor, rules.clone());
        rules
    }

    /// Activate the bridge and bind connected clients.
    pub fn activate(&mut self) -> Result<(), PickupError> {
        self.operations.assert_current()?;
        self.active = true;
        for actor in self.operations.client_actors() {
            self.bind_actor(actor)?;
        }
        Ok(())
    }

    /// Bind inventory delegates for one actor.
    pub fn bind_actor(&mut self, actor: NativeActorId) -> Result<(), PickupError> {
        if !self.active || !self.operations.is_eligible(actor) || self.delegates.contains_key(&actor) {
            return Ok(());
        }
        if !self.operations.is_live(actor) {
            return Err(PickupError::RecipientRetired);
        }
        let rules: Vec<PickupRule> = self
            .actor_rules(actor)
            .into_iter()
            .filter(|rule| {
                rule.writes
                    .iter()
                    .any(|write| matches!(write, PickupWrite::Inventory { .. }))
            })
            .collect();
        if rules.is_empty() {
            self.delegates.insert(actor, Vec::new());
            return Ok(());
        }
        let ids: Vec<String> = rules.iter().map(|rule| rule.id.clone()).collect();
        let token = self.operations.bind_pickup_rules(actor, &ids);
        self.delegates.insert(actor, vec![token]);
        Ok(())
    }

    /// Take one offer through the named rule.
    pub fn take(
        &mut self,
        actor: NativeActorId,
        rule_id: &str,
        offer: &PickupOffer,
        execution: &dyn PickupExecution,
    ) -> Result<PickupDecision, PickupError> {
        self.operations.assert_current()?;
        let definition = self
            .definitions
            .iter()
            .find(|definition| definition.id == rule_id)
            .cloned()
            .ok_or(PickupError::StaleRule)?;
        if offer.recipient != actor
            || !definition.offered.contains(&offer.item)
            || !is_current(&self.operations, execution, self.active, actor, offer.pickup)
        {
            return Err(PickupError::StaleRule);
        }
        let mut values = HashMap::new();
        values.insert("self".to_string(), RuntimeValue::Actor(Some(actor)));
        values.insert("other".to_string(), RuntimeValue::Actor(Some(offer.pickup)));
        values.insert("item".to_string(), RuntimeValue::Text(offer.item.clone()));
        values.insert("time".to_string(), RuntimeValue::Float(offer.time_secs));
        values.insert(
            "pickup-count".to_string(),
            RuntimeValue::Float(offer.count_override.unwrap_or(0.0)),
        );
        values.insert(
            "pickup-has-count".to_string(),
            RuntimeValue::Float(f64::from(offer.count_override.is_some())),
        );
        values.insert(
            "pickup-dropped".to_string(),
            RuntimeValue::Float(f64::from(offer.dropped)),
        );
        let inputs = PickupInputs { values };
        let active = self.active;
        self.depth += 1;
        let outcome = self.operations.context(&definition, offer, &inputs, |operations| {
            operations.observe(actor, execution, |operations| {
                let operation = definition.operation.clone();
                match operation {
                    PickupOperation::GateThenGrant {
                        gate,
                        grant,
                        grant_accepts,
                    } => {
                        let gate = operations.invoke(&gate, &inputs);
                        if gate.is_none_or(|value| value == 0)
                            || !is_current(operations, execution, active, actor, offer.pickup)
                        {
                            return PickupDecision::Refused;
                        }
                        let result = operations.invoke(&grant, &inputs);
                        if result.is_none() || !is_current(operations, execution, active, actor, offer.pickup) {
                            return PickupDecision::Refused;
                        }
                        if grant_accepts == GrantAccepts::Always || result != Some(0) {
                            PickupDecision::Accepted
                        } else {
                            PickupDecision::Refused
                        }
                    }
                    PickupOperation::BooleanGrant { grant } => {
                        let result = operations.invoke(&grant, &inputs);
                        if result.is_none() || !is_current(operations, execution, active, actor, offer.pickup) {
                            return PickupDecision::Refused;
                        }
                        if result != Some(0) {
                            PickupDecision::Accepted
                        } else {
                            PickupDecision::Refused
                        }
                    }
                }
            })
        });
        self.depth -= 1;
        Ok(outcome)
    }

    /// Assert no pickup execution is in flight.
    pub fn assert_idle(&self) -> Result<(), PickupError> {
        if self.depth != 0 {
            return Err(PickupError::NotCurrent(
                "cannot save or restore during native original pickup execution".to_string(),
            ));
        }
        Ok(())
    }

    /// Release one actor's delegates and cached rules.
    pub fn release(&mut self, actor: NativeActorId) -> Result<(), PickupError> {
        let tokens = self.delegates.remove(&actor).unwrap_or_default();
        self.rules.remove(&actor);
        let mut errors = Vec::new();
        for token in tokens {
            if let Err(error) = self.operations.unbind_pickup_rules(token) {
                errors.push(error.to_string());
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(PickupError::Cleanup(errors))
        }
    }

    /// Release every delegate.
    pub fn close(&mut self) -> Result<(), PickupError> {
        self.active = false;
        self.rules.clear();
        let mut errors = Vec::new();
        for actor in self.delegates.keys().copied().collect::<Vec<_>>() {
            if let Err(error) = self.release(actor) {
                errors.push(error.to_string());
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(PickupError::Cleanup(errors))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(slot: u32) -> NativeActorId {
        NativeActorId { slot, generation: 1 }
    }

    fn gate_definition() -> PickupDefinition {
        PickupDefinition {
            id: "health".to_string(),
            offered: vec!["q2:health".to_string()],
            writes: vec![PickupWrite::Inventory {
                item: "q2:health".to_string(),
                fields: PickupFields::Count,
            }],
            operation: PickupOperation::GateThenGrant {
                gate: PickupCall {
                    id: "gate".to_string(),
                    returns_value: true,
                },
                grant: PickupCall {
                    id: "grant".to_string(),
                    returns_value: true,
                },
                grant_accepts: GrantAccepts::NonZero,
            },
        }
    }

    struct TestExecution {
        current: bool,
        writes: Vec<PickupWrite>,
    }

    impl PickupExecution for TestExecution {
        fn is_current(&self) -> bool {
            self.current
        }

        fn writes(&self) -> &[PickupWrite] {
            &self.writes
        }
    }

    struct TestOps {
        live: HashSet<NativeActorId>,
        eligible: HashSet<NativeActorId>,
        clients: Vec<NativeActorId>,
        results: HashMap<String, Option<i32>>,
        invoked: Vec<String>,
        seen_inputs: Vec<PickupInputs>,
        bindings: Vec<(NativeActorId, Vec<String>)>,
        unbind_errors: HashSet<u64>,
        next_token: u64,
    }

    impl PickupOperations for TestOps {
        fn assert_current(&self) -> Result<(), PickupError> {
            Ok(())
        }

        fn is_live(&self, actor: NativeActorId) -> bool {
            self.live.contains(&actor)
        }

        fn is_eligible(&self, actor: NativeActorId) -> bool {
            self.eligible.contains(&actor)
        }

        fn client_actors(&self) -> Vec<NativeActorId> {
            self.clients.clone()
        }

        fn context<R>(
            &mut self,
            _definition: &PickupDefinition,
            _offer: &PickupOffer,
            _inputs: &PickupInputs,
            execute: impl FnOnce(&mut Self) -> R,
        ) -> R {
            execute(self)
        }

        fn observe<R>(
            &mut self,
            _actor: NativeActorId,
            execution: &dyn PickupExecution,
            execute: impl FnOnce(&mut Self) -> R,
        ) -> R {
            assert!(execution.is_current());
            execute(self)
        }

        fn invoke(&mut self, call: &PickupCall, inputs: &PickupInputs) -> Option<i32> {
            self.invoked.push(call.id.clone());
            self.seen_inputs.push(inputs.clone());
            self.results.get(&call.id).copied().unwrap_or(Some(1))
        }

        fn bind_pickup_rules(&mut self, actor: NativeActorId, rule_ids: &[String]) -> u64 {
            self.bindings.push((actor, rule_ids.to_vec()));
            self.next_token += 1;
            self.next_token
        }

        fn unbind_pickup_rules(&mut self, token: u64) -> Result<(), PickupError> {
            if self.unbind_errors.contains(&token) {
                return Err(PickupError::NotCurrent("unbind failed".to_string()));
            }
            Ok(())
        }
    }

    fn fixture() -> NativeModPickups<TestOps> {
        NativeModPickups::new(
            vec![gate_definition()],
            "test:mod",
            TestOps {
                live: [actor(1), actor(2)].into_iter().collect(),
                eligible: [actor(1)].into_iter().collect(),
                clients: vec![actor(1)],
                results: HashMap::new(),
                invoked: Vec::new(),
                seen_inputs: Vec::new(),
                bindings: Vec::new(),
                unbind_errors: HashSet::new(),
                next_token: 0,
            },
        )
    }

    fn offer() -> PickupOffer {
        PickupOffer {
            recipient: actor(1),
            pickup: actor(2),
            item: "q2:health".to_string(),
            count_override: Some(25.0),
            dropped: true,
            time_secs: 12.5,
        }
    }

    fn execution() -> TestExecution {
        TestExecution {
            current: true,
            writes: vec![PickupWrite::Inventory {
                item: "q2:health".to_string(),
                fields: PickupFields::Count,
            }],
        }
    }

    #[test]
    fn gate_then_grant_paths_and_inputs() {
        let mut bridge = fixture();
        bridge.activate().unwrap();
        assert_eq!(bridge.operations().bindings.len(), 1);
        let decision = bridge.take(actor(1), "health", &offer(), &execution()).unwrap();
        assert_eq!(decision, PickupDecision::Accepted);
        assert_eq!(
            bridge.operations().invoked,
            vec!["gate".to_string(), "grant".to_string()]
        );
        let inputs = &bridge.operations().seen_inputs[0];
        assert_eq!(inputs.get("pickup-count"), Some(&RuntimeValue::Float(25.0)));
        assert_eq!(inputs.get("pickup-dropped"), Some(&RuntimeValue::Float(1.0)));
        assert_eq!(inputs.get("item"), Some(&RuntimeValue::Text("q2:health".to_string())));

        bridge.operations_mut().results.insert("gate".to_string(), Some(0));
        let refused = bridge.take(actor(1), "health", &offer(), &execution()).unwrap();
        assert_eq!(refused, PickupDecision::Refused);

        bridge.operations_mut().results.insert("gate".to_string(), Some(1));
        bridge.operations_mut().results.insert("grant".to_string(), Some(0));
        let zero = bridge.take(actor(1), "health", &offer(), &execution()).unwrap();
        assert_eq!(zero, PickupDecision::Refused);
        assert!(!bridge.in_progress());
        bridge.assert_idle().unwrap();
    }

    #[test]
    fn stale_offers_fail_and_lifecycle_aggregates() {
        let mut bridge = fixture();
        bridge.activate().unwrap();
        let mut foreign = offer();
        foreign.recipient = actor(2);
        assert_eq!(
            bridge.take(actor(1), "health", &foreign, &execution()),
            Err(PickupError::StaleRule)
        );
        assert_eq!(
            bridge.take(actor(1), "missing", &offer(), &execution()),
            Err(PickupError::StaleRule)
        );
        bridge.release(actor(1)).unwrap();
        bridge.bind_actor(actor(1)).unwrap();
        bridge.operations_mut().unbind_errors.insert(2);
        let closed = bridge.close();
        assert!(matches!(closed, Err(PickupError::Cleanup(_))));
    }

    #[test]
    fn declaration_validation_catches_bad_rules() {
        assert!(validate_native_mod_pickups(&[gate_definition()], true, &[], &["q2:health".to_string()], &[],).is_ok());
        let mut duplicated = gate_definition();
        duplicated.id = String::new();
        assert_eq!(
            validate_native_mod_pickups(&[duplicated], true, &[], &["q2:health".to_string()], &[],),
            Err(PickupError::BadRules)
        );
        let mut unowned = gate_definition();
        unowned.writes = vec![PickupWrite::Protection {
            channel: ProtectionChannel::Powered,
        }];
        assert_eq!(
            validate_native_mod_pickups(&[unowned], true, &[ProtectionChannel::Regular], &[], &[],),
            Err(PickupError::MissingProtection)
        );
        let mut void_grant = gate_definition();
        void_grant.operation = PickupOperation::BooleanGrant {
            grant: PickupCall {
                id: "grant".to_string(),
                returns_value: false,
            },
        };
        assert_eq!(
            validate_native_mod_pickups(&[void_grant], true, &[], &["q2:health".to_string()], &[],),
            Err(PickupError::MissingDecision)
        );
    }
}
