//! QuakeC pickup rules over the shared inventory binding.
//!
//! Ported from `src/compat/qc/mod-pickups.ts`.
//!
//! Local mirrors (owned by other workers' modules, noted here):
//! `QcPickupServices` mirrors the actor/client/inventory surface of
//! `ModHostServices` from `src/world/session/mods.ts`; `PickupOffer`,
//! `PickupCount`, `PickupDecision`, and `PickupExecution` mirror
//! `src/contracts/original-pickups.ts`; `QcPickupDispatch` mirrors the
//! invoke/watch operations. Declaration mirrors (`ModPickupRule`,
//! `PickupWrite`, `PickupFields`, `PickupOperation`, `GrantAccepts`) live in
//! `super::mod_provider`.

use std::collections::HashMap;

use qa_core::identity::{ActorId, ClientId, OwnedActor, ProviderId};
use qa_core::time::SourceTime;

use super::mod_provider::{
    GrantAccepts, ItemId, ModCallbackDeclaration, ModCallbackInput, ModPickupRule, ModRuntimeValue, ModSourceCall,
    PickupOperation, PickupWrite, ProtectionChannel, QcModInputs,
};
use crate::error::GuestError;

/// Pickup count override.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PickupCount {
    /// Default count.
    Default,
    /// Override amount.
    Override(f64),
}

/// Pickup offer.
#[derive(Debug, Clone, PartialEq)]
pub struct PickupOffer {
    /// Recipient actor.
    pub recipient: ActorId,
    /// Pickup actor.
    pub pickup: ActorId,
    /// Source provider.
    pub source: ProviderId,
    /// Offered item.
    pub item: ItemId,
    /// Count override.
    pub count: PickupCount,
    /// Whether the pickup was dropped.
    pub dropped: bool,
    /// Offer time.
    pub time: SourceTime,
}

/// Pickup decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PickupDecision {
    /// Grant accepted.
    Accepted,
    /// Grant refused.
    Refused,
}

/// Live pickup execution observed during a grant.
pub trait PickupExecution {
    /// Declared writes.
    fn writes(&self) -> &[PickupWrite];
    /// Whether the binding is current.
    fn current(&self) -> bool;
}

/// Runtime pickup rule bound to one actor.
#[derive(Debug, Clone, PartialEq)]
pub struct QcPickupRule {
    /// Bound actor.
    pub actor: ActorId,
    /// Rule identifier.
    pub id: String,
    /// Offered items.
    pub offered: Vec<ItemId>,
    /// Declared writes.
    pub writes: Vec<PickupWrite>,
    /// Operation.
    pub operation: PickupOperation,
}

/// Host services for pickups.
pub trait QcPickupServices {
    /// Resolve an owned actor.
    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor>;
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Client handle for an actor.
    fn client_for_actor(&self, actor: &ActorId) -> Option<ClientId>;
    /// Actor for a client handle.
    fn actor_for_client(&self, client: &ClientId) -> Option<ActorId>;
    /// Bind inventory pickup rules; returns a binding id.
    fn bind_pickup(
        &mut self,
        owner: &OwnedActor,
        provider: &ProviderId,
        rules: &[QcPickupRule],
    ) -> Result<u64, GuestError>;
    /// Release an inventory pickup binding.
    fn unbind_pickup(&mut self, binding: u64) -> Result<(), GuestError>;
}

/// Source dispatch for pickups.
pub trait QcPickupDispatch {
    /// Invoke a source call.
    fn invoke(&mut self, call: &ModSourceCall, inputs: &QcModInputs) -> Result<f64, GuestError>;
    /// Watch protection stores while running a grant.
    fn watch(
        &mut self,
        _actor: &ActorId,
        _execution: &dyn PickupExecution,
        run: &mut dyn FnMut(&mut Self) -> Result<PickupDecision, GuestError>,
    ) -> Result<PickupDecision, GuestError> {
        run(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    client: ClientId,
    bindings: Vec<u64>,
}

/// Pickup rules for admitted clients.
pub struct QcModPickups<S, D> {
    declaration: ModCallbackDeclaration,
    provider: ProviderId,
    services: S,
    dispatch: D,
    rules: HashMap<ActorId, Vec<QcPickupRule>>,
    entries: HashMap<ActorId, Entry>,
}

impl<S: QcPickupServices, D: QcPickupDispatch> QcModPickups<S, D> {
    /// Build over a declaration and host services.
    pub fn new(declaration: ModCallbackDeclaration, provider: ProviderId, services: S, dispatch: D) -> Self {
        Self {
            declaration,
            provider,
            services,
            dispatch,
            rules: HashMap::new(),
            entries: HashMap::new(),
        }
    }

    /// Borrow the services.
    #[must_use]
    pub fn services(&self) -> &S {
        &self.services
    }

    /// Borrow the dispatch.
    #[must_use]
    pub fn dispatch(&self) -> &D {
        &self.dispatch
    }

    /// Admit a client actor.
    pub fn admit(&mut self, actor: &ActorId) -> Result<(), GuestError> {
        if self.entries.contains_key(actor) {
            return Ok(());
        }
        let owned = self.services.resolve_owned(actor);
        let client = self.services.client_for_actor(actor);
        let (Some(owner), Some(client)) = (owned, client) else {
            return Err(GuestError::invalid("QC pickups require a live canonical client"));
        };
        if self.services.actor_for_client(&client).as_ref() != Some(actor) {
            return Err(GuestError::invalid("QC pickups require a live canonical client"));
        }
        let mut entry = Entry {
            client,
            bindings: Vec::new(),
        };
        self.entries.insert(actor.clone(), entry.clone());
        let rules: Vec<QcPickupRule> = self
            .actor_rules(actor)
            .into_iter()
            .filter(|rule| {
                rule.writes
                    .iter()
                    .any(|write| matches!(write, PickupWrite::Inventory { .. }))
            })
            .collect();
        if !rules.is_empty() {
            match self.services.bind_pickup(&owner, &self.provider, &rules) {
                Ok(binding) => {
                    entry.bindings.push(binding);
                    self.entries.insert(actor.clone(), entry);
                }
                Err(error) => {
                    let cleanup = self.release(actor).err().map(|cleanup| cleanup.to_string());
                    let mut causes = vec![error.to_string()];
                    causes.extend(cleanup);
                    return Err(GuestError::Callback(format!(
                        "QC pickup admission failed: {}",
                        causes.join("; ")
                    )));
                }
            }
        }
        Ok(())
    }

    /// Rules bound to one actor.
    pub fn actor_rules(&mut self, actor: &ActorId) -> Vec<QcPickupRule> {
        self.rules
            .entry(actor.clone())
            .or_insert_with_key(|actor| {
                self.declaration
                    .pickups
                    .iter()
                    .map(|definition| Self::rule(actor, definition))
                    .collect()
            })
            .clone()
    }

    /// Protection rules for one channel.
    pub fn protection(&mut self, actor: &ActorId, channel: ProtectionChannel) -> Vec<QcPickupRule> {
        self.actor_rules(actor)
            .into_iter()
            .filter(|rule| {
                rule.writes
                    .iter()
                    .any(|write| matches!(write, PickupWrite::Protection { channel: bound } if *bound == channel))
            })
            .collect()
    }

    /// Build one runtime rule.
    fn rule(actor: &ActorId, definition: &ModPickupRule) -> QcPickupRule {
        QcPickupRule {
            actor: actor.clone(),
            id: definition.id.clone(),
            offered: definition.offered.clone(),
            writes: definition.writes.clone(),
            operation: definition.operation.clone(),
        }
    }

    /// Whether an entry is current.
    fn current(services: &S, entries: &HashMap<ActorId, Entry>, actor: &ActorId, entry: &Entry) -> bool {
        entries.get(actor) == Some(entry)
            && services.is_live(actor)
            && services.client_for_actor(actor).as_ref() == Some(&entry.client)
            && services.actor_for_client(&entry.client).as_ref() == Some(actor)
    }

    /// Take a pickup offer through one rule.
    pub fn take(
        &mut self,
        rule: &QcPickupRule,
        offer: &PickupOffer,
        execution: &dyn PickupExecution,
    ) -> Result<PickupDecision, GuestError> {
        let actor = rule.actor.clone();
        let entry = self.entries.get(&actor).cloned();
        let admitted = self
            .rules
            .get(&actor)
            .is_some_and(|rules| rules.iter().any(|bound| bound.id == rule.id));
        let Some(entry) = entry else {
            return Err(GuestError::invalid(
                "QC pickup invocation differs from its admitted source binding",
            ));
        };
        if !admitted
            || !execution.current()
            || !Self::current(&self.services, &self.entries, &actor, &entry)
            || offer.recipient != actor
            || !self.services.is_live(&offer.pickup)
            || !rule.offered.contains(&offer.item)
        {
            return Err(GuestError::invalid(
                "QC pickup invocation differs from its admitted source binding",
            ));
        }
        let count = match offer.count {
            PickupCount::Override(amount) => amount,
            PickupCount::Default => 0.0,
        };
        let time = offer.time.as_seconds_f64();
        if !is_binary32(count) || !is_binary32(time) {
            return Err(GuestError::invalid("QC pickup input exceeds its source scalar ABI"));
        }
        let mut inputs = QcModInputs::new();
        inputs.insert(ModCallbackInput::Self_, ModRuntimeValue::Actor(Some(actor.clone())));
        inputs.insert(
            ModCallbackInput::Other,
            ModRuntimeValue::Actor(Some(offer.pickup.clone())),
        );
        inputs.insert(ModCallbackInput::Item, ModRuntimeValue::String(offer.item.clone()));
        inputs.insert(ModCallbackInput::Time, ModRuntimeValue::Float(time));
        inputs.insert(ModCallbackInput::PickupCount, ModRuntimeValue::Float(count));
        inputs.insert(
            ModCallbackInput::PickupHasCount,
            ModRuntimeValue::Float(if matches!(offer.count, PickupCount::Override(_)) {
                1.0
            } else {
                0.0
            }),
        );
        inputs.insert(
            ModCallbackInput::PickupDropped,
            ModRuntimeValue::Float(if offer.dropped { 1.0 } else { 0.0 }),
        );
        let operation = rule.operation.clone();
        let pickup = offer.pickup.clone();
        let Self {
            services,
            entries,
            dispatch,
            ..
        } = self;
        let (services_shared, entries_shared) = (&*services, &*entries);
        dispatch.watch(&actor, execution, &mut |dispatch| {
            let live = || {
                execution.current()
                    && Self::current(services_shared, entries_shared, &actor, &entry)
                    && services_shared.is_live(&pickup)
            };
            match &operation {
                PickupOperation::BooleanGrant { grant } => Ok(if accepts(dispatch.invoke(grant, &inputs)?)? {
                    PickupDecision::Accepted
                } else {
                    PickupDecision::Refused
                }),
                PickupOperation::GateThenGrant {
                    gate,
                    grant,
                    grant_accepts,
                } => {
                    if !accepts(dispatch.invoke(gate, &inputs)?)? || !live() {
                        return Ok(PickupDecision::Refused);
                    }
                    let result = dispatch.invoke(grant, &inputs)?;
                    if *grant_accepts == GrantAccepts::Always || accepts(result)? {
                        Ok(PickupDecision::Accepted)
                    } else {
                        Ok(PickupDecision::Refused)
                    }
                }
            }
        })
    }

    /// Release a client actor.
    pub fn release(&mut self, actor: &ActorId) -> Result<(), GuestError> {
        let entry = match self.entries.remove(actor) {
            Some(entry) => entry,
            None => return Ok(()),
        };
        self.rules.remove(actor);
        let mut errors = Vec::new();
        for binding in entry.bindings {
            if let Err(error) = self.services.unbind_pickup(binding) {
                errors.push(error.to_string());
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(GuestError::Callback(format!(
                "QC pickup release failed: {}",
                errors.join("; ")
            )))
        }
    }

    /// Release all client actors.
    pub fn close(&mut self) -> Result<(), GuestError> {
        self.rules.clear();
        let mut errors = Vec::new();
        let actors: Vec<ActorId> = self.entries.keys().cloned().collect();
        for actor in actors {
            if let Err(error) = self.release(&actor) {
                errors.push(error.to_string());
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(GuestError::Callback(format!(
                "QC pickup close failed: {}",
                errors.join("; ")
            )))
        }
    }
}

/// Whether a scalar survives the binary32 source ABI.
fn is_binary32(value: f64) -> bool {
    value.is_finite() && f64::from(value as f32) == value
}

/// Whether a source result accepts a grant.
fn accepts(value: f64) -> Result<bool, GuestError> {
    if !value.is_finite() {
        return Err(GuestError::invalid("QC pickup returned a non-finite source result"));
    }
    Ok(value != 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::time::SourceTime;

    use super::super::mod_provider::PickupFields;

    struct FakeServices {
        owner: IdentityOwner,
        live: Vec<ActorId>,
        clients: HashMap<ActorId, ClientId>,
        bindings: HashMap<u64, Vec<QcPickupRule>>,
        next_binding: u64,
        unbound: Vec<u64>,
    }

    impl QcPickupServices for FakeServices {
        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            self.live
                .iter()
                .find(|live| *live == actor)
                .and_then(|actor| self.owner.owned_actor(actor, ProviderId::new("mod", "test")).ok())
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            self.live.contains(actor)
        }

        fn client_for_actor(&self, actor: &ActorId) -> Option<ClientId> {
            self.clients.get(actor).cloned()
        }

        fn actor_for_client(&self, client: &ClientId) -> Option<ActorId> {
            self.clients
                .iter()
                .find(|(_, bound)| *bound == client)
                .map(|(actor, _)| actor.clone())
        }

        fn bind_pickup(
            &mut self,
            _owner: &OwnedActor,
            _provider: &ProviderId,
            rules: &[QcPickupRule],
        ) -> Result<u64, GuestError> {
            let id = self.next_binding;
            self.next_binding += 1;
            self.bindings.insert(id, rules.to_vec());
            Ok(id)
        }

        fn unbind_pickup(&mut self, binding: u64) -> Result<(), GuestError> {
            self.bindings
                .remove(&binding)
                .ok_or_else(|| GuestError::invalid("Unknown pickup binding"))?;
            self.unbound.push(binding);
            Ok(())
        }
    }

    struct FakeDispatch {
        results: Vec<f64>,
        calls: Vec<String>,
        watched: usize,
    }

    impl QcPickupDispatch for FakeDispatch {
        fn invoke(&mut self, call: &ModSourceCall, inputs: &QcModInputs) -> Result<f64, GuestError> {
            self.calls.push(format!(
                "{} self={:?}",
                call.function,
                inputs.get(&ModCallbackInput::Self_)
            ));
            Ok(self.results.pop().unwrap_or(0.0))
        }

        fn watch(
            &mut self,
            _actor: &ActorId,
            _execution: &dyn PickupExecution,
            run: &mut dyn FnMut(&mut Self) -> Result<PickupDecision, GuestError>,
        ) -> Result<PickupDecision, GuestError> {
            self.watched += 1;
            run(self)
        }
    }

    struct FakeExecution {
        writes: Vec<PickupWrite>,
        live: bool,
    }

    impl PickupExecution for FakeExecution {
        fn writes(&self) -> &[PickupWrite] {
            &self.writes
        }

        fn current(&self) -> bool {
            self.live
        }
    }

    fn gate_grant_rule() -> ModPickupRule {
        ModPickupRule {
            id: "shells".to_string(),
            writes: vec![PickupWrite::Inventory {
                item: "q1:item_shells".to_string(),
                fields: PickupFields::Count,
            }],
            offered: vec!["q1:item_shells".to_string()],
            operation: PickupOperation::GateThenGrant {
                gate: ModSourceCall {
                    function: "gate".to_string(),
                    arguments: vec![],
                    globals: vec![],
                },
                grant: ModSourceCall {
                    function: "grant".to_string(),
                    arguments: vec![],
                    globals: vec![],
                },
                grant_accepts: GrantAccepts::Nonzero,
            },
        }
    }

    fn fixture() -> QcModPickups<FakeServices, FakeDispatch> {
        let declaration = ModCallbackDeclaration {
            pickups: vec![gate_grant_rule()],
            ..ModCallbackDeclaration::default()
        };
        let services = FakeServices {
            owner: IdentityOwner::create("pickups").unwrap(),
            live: Vec::new(),
            clients: HashMap::new(),
            bindings: HashMap::new(),
            next_binding: 1,
            unbound: Vec::new(),
        };
        QcModPickups::new(
            declaration,
            ProviderId::new("mod", "test"),
            services,
            FakeDispatch {
                results: Vec::new(),
                calls: Vec::new(),
                watched: 0,
            },
        )
    }

    fn join(pickups: &mut QcModPickups<FakeServices, FakeDispatch>) -> (ActorId, ActorId) {
        let recipient = pickups.services.owner.actor(2, 1);
        let offered = pickups.services.owner.actor(9, 1);
        let client = pickups.services.owner.client(2, 1);
        pickups.services.live = vec![recipient.clone(), offered.clone()];
        pickups.services.clients.insert(recipient.clone(), client);
        (recipient, offered)
    }

    #[test]
    fn admit_binds_inventory_rules() {
        let mut pickups = fixture();
        let (recipient, _) = join(&mut pickups);
        pickups.admit(&recipient).unwrap();
        assert_eq!(pickups.services.bindings.len(), 1);
        assert_eq!(pickups.actor_rules(&recipient).len(), 1);
        assert!(pickups.protection(&recipient, ProtectionChannel::Regular).is_empty());
        pickups.release(&recipient).unwrap();
        assert_eq!(pickups.services.unbound.len(), 1);
    }

    #[test]
    fn gate_then_grant_accepts() {
        let mut pickups = fixture();
        let (recipient, offered) = join(&mut pickups);
        pickups.admit(&recipient).unwrap();
        pickups.dispatch.results = vec![1.0, 1.0];
        let rule = pickups.actor_rules(&recipient).pop().unwrap();
        let execution = FakeExecution {
            writes: rule.writes.clone(),
            live: true,
        };
        let offer = PickupOffer {
            recipient: recipient.clone(),
            pickup: offered,
            source: ProviderId::new("mod", "test"),
            item: "q1:item_shells".to_string(),
            count: PickupCount::Override(5.0),
            dropped: false,
            time: SourceTime::Seconds(3.0),
        };
        assert_eq!(
            pickups.take(&rule, &offer, &execution).unwrap(),
            PickupDecision::Accepted
        );
        assert_eq!(pickups.dispatch.watched, 1);
        assert_eq!(pickups.dispatch.calls.len(), 2);
        assert_eq!(execution.writes(), rule.writes.as_slice());
    }

    #[test]
    fn refused_gate_refuses_without_grant() {
        let mut pickups = fixture();
        let (recipient, offered) = join(&mut pickups);
        pickups.admit(&recipient).unwrap();
        pickups.dispatch.results = vec![0.0];
        let rule = pickups.actor_rules(&recipient).pop().unwrap();
        let execution = FakeExecution {
            writes: rule.writes.clone(),
            live: true,
        };
        let offer = PickupOffer {
            recipient: recipient.clone(),
            pickup: offered,
            source: ProviderId::new("mod", "test"),
            item: "q1:item_shells".to_string(),
            count: PickupCount::Default,
            dropped: true,
            time: SourceTime::Seconds(3.0),
        };
        assert_eq!(
            pickups.take(&rule, &offer, &execution).unwrap(),
            PickupDecision::Refused
        );
        assert_eq!(pickups.dispatch.calls.len(), 1);
    }

    #[test]
    fn stale_or_foreign_offers_error() {
        let mut pickups = fixture();
        let (recipient, offered) = join(&mut pickups);
        pickups.admit(&recipient).unwrap();
        let rule = pickups.actor_rules(&recipient).pop().unwrap();
        let foreign = pickups.services.owner.actor(7, 1);
        let execution = FakeExecution {
            writes: rule.writes.clone(),
            live: false,
        };
        let offer = PickupOffer {
            recipient: foreign,
            pickup: offered.clone(),
            source: ProviderId::new("mod", "test"),
            item: "q1:item_shells".to_string(),
            count: PickupCount::Default,
            dropped: false,
            time: SourceTime::Seconds(3.0),
        };
        assert!(pickups.take(&rule, &offer, &execution).is_err());
        let live_execution = FakeExecution {
            writes: rule.writes.clone(),
            live: true,
        };
        let wrong_item = PickupOffer {
            recipient: recipient.clone(),
            pickup: offered,
            item: "q1:item_nails".to_string(),
            ..offer.clone()
        };
        assert!(pickups.take(&rule, &wrong_item, &live_execution).is_err());
    }

    #[test]
    fn close_releases_all() {
        let mut pickups = fixture();
        let (recipient, _) = join(&mut pickups);
        pickups.admit(&recipient).unwrap();
        pickups.close().unwrap();
        assert!(pickups.services.bindings.is_empty());
        pickups.close().unwrap();
    }
}
