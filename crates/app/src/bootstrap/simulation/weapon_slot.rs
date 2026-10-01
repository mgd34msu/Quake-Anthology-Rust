//! One actor's source weapon selection; each bound source owns its traversal.
//! Donor: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/weapon-slot.ts`.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_content::contract::ItemId;
use qa_core::identity::ProviderId;
use thiserror::Error;

/// Weapon owned by one source provider.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WeaponReference {
    /// Owning source provider.
    pub provider: ProviderId,
    /// Weapon item.
    pub item: ItemId,
}

/// Deferred source-input request status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestStatus {
    /// Still pending.
    Pending,
    /// Accepted.
    Accepted,
    /// Refused.
    Refused,
}

/// Deferred source resumption/activation request.
pub trait SourceWeaponRequest {
    /// Stable request id.
    fn id(&self) -> u64;
    /// Current status.
    fn status(&mut self) -> RequestStatus;
    /// Cancel the request.
    fn cancel(&mut self);
}

/// Outcome of resuming a source handoff.
pub enum ResumeOutcome {
    /// Immediate accept/refuse.
    Immediate(bool),
    /// Deferred source-input request.
    Deferred(Box<dyn SourceWeaponRequest>),
}

/// Source-owned weapon selection; traversal stays with the source.
pub trait SourceWeaponHandoff {
    /// Owning provider.
    fn provider(&self) -> ProviderId;
    /// Whether the source admits the weapon.
    fn accepts(&self, item: &ItemId) -> bool;
    /// Select within the already-active source.
    fn select(&mut self, item: &ItemId) -> bool;
    /// Start holstering.
    fn holster(&mut self);
    /// Whether fully holstered.
    fn is_holstered(&self) -> bool;
    /// Resume this source, optionally onto an item.
    fn resume(&mut self, item: Option<&ItemId>) -> ResumeOutcome;
    /// Rebuild a saved deferred request. Immediate handoffs return a refused request.
    fn restore_request(&mut self, id: u64, item: Option<&ItemId>) -> Box<dyn SourceWeaponRequest>;
    /// Whether resumption defers through a source-input request.
    fn is_deferred(&self) -> bool;
}

/// Primary arsenal handoff (donor re-export name).
pub trait PrimaryWeaponHandoff: SourceWeaponHandoff {}
impl<T: SourceWeaponHandoff + ?Sized> PrimaryWeaponHandoff for T {}

/// Equipment-owned weapon (grenades); wrapped as an immediate handoff.
pub trait EquipmentWeaponHandoff {
    /// Owned weapon.
    fn weapon(&self) -> &WeaponReference;
    /// Holster.
    fn holster(&mut self);
    /// Whether holstered.
    fn is_holstered(&self) -> bool;
    /// Resume.
    fn resume(&mut self);
}

/// Absorbed minimal source presentation (donor `SourceWeaponPresentation` projection).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceWeaponPresentation {
    /// Presenting provider; must equal the binding owner.
    pub source_provider: ProviderId,
    /// Active weapon, must be declared in `items`.
    pub active: Option<ItemId>,
    /// Declared weapon items.
    pub items: Vec<WeaponItemDeclaration>,
}

/// Absorbed minimal weapon item declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponItemDeclaration {
    /// Weapon item.
    pub item: ItemId,
}

/// Live slot state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WeaponSlotState {
    /// Source selected.
    Active {
        /// Active provider.
        provider: ProviderId,
    },
    /// Holstering `from` before selecting `next`.
    Switching {
        /// Outgoing provider.
        from: ProviderId,
        /// Incoming weapon.
        next: WeaponReference,
    },
    /// Deferred activation in flight.
    Activating {
        /// Outgoing provider.
        from: ProviderId,
        /// Incoming weapon.
        next: WeaponReference,
        /// Deferred request id.
        request: u64,
    },
}

/// Restorable slot state, including legacy lowered forms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WeaponSlotRestoreState {
    /// Modern state.
    State(WeaponSlotState),
    /// Legacy primary selection.
    Primary,
    /// Legacy equipment selection.
    Equipment,
    /// Legacy holster off primary.
    HolsteringPrimary {
        /// Incoming weapon.
        next: WeaponReference,
    },
    /// Legacy holster off equipment.
    HolsteringEquipment {
        /// Incoming weapon.
        next: WeaponReference,
    },
}

/// Slot failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WeaponSlotError {
    /// Equipment and primary weapon owners overlap.
    #[error("Equipment and primary weapon owners overlap")]
    OwnerOverlap,
    /// Saved equipment weapon has no owner.
    #[error("Saved equipment weapon has no owner")]
    EquipmentWithoutOwner,
    /// Saved equipment transition has no owner.
    #[error("Saved equipment transition has no owner")]
    EquipmentTransitionWithoutOwner,
    /// Saved weapon source is not bound.
    #[error("Saved weapon source is not bound")]
    RestoreUnbound,
    /// Saved active weapon is not declared by its source owner.
    #[error("Saved active weapon is not declared by its source owner")]
    RestoreUndeclared,
    /// Saved pending weapon is not admitted to its source owner.
    #[error("Saved pending weapon is not admitted to its source owner")]
    RestoreUnadmitted,
    /// Saved activation requires its original input owner.
    #[error("Saved activation requires its original input owner")]
    RestoreNeedsInputOwner,
    /// Saved weapon activation changed during restoration.
    #[error("Saved weapon activation changed during restoration")]
    RestoreChanged,
    /// Source weapon owner is unavailable or already bound.
    #[error("Source weapon owner is unavailable or already bound")]
    BindUnavailable,
    /// Source weapon presentation belongs to another owner.
    #[error("Source weapon presentation belongs to another owner")]
    ForeignPresentation,
    /// Weapon selection has no live source owner.
    #[error("Weapon selection has no live source owner")]
    NoLiveOwner,
    /// Deferred source resumption has no unchanged outgoing owner.
    #[error("Deferred source resumption has no unchanged outgoing owner")]
    ResumeWithoutOutgoing,
    /// Original source refused weapon resumption.
    #[error("Original source refused weapon resumption")]
    ResumeRefused,
    /// Weapon activation lost its source request.
    #[error("Weapon activation lost its source request")]
    ActivationLost,
}

/// Source presentation reader.
pub type WeaponReadFn = Box<dyn Fn() -> SourceWeaponPresentation>;

/// Token returned by [`WeaponSlot::bind`]; pass to [`WeaponSlot::unbind`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingToken {
    /// Bound provider.
    pub provider: ProviderId,
    /// Binding generation.
    pub generation: u64,
}

struct Binding {
    handoff: Box<dyn SourceWeaponHandoff>,
    live: Rc<Cell<bool>>,
    read: Option<WeaponReadFn>,
    generation: u64,
}

struct Activation {
    state: WeaponSlotState,
    request: Box<dyn SourceWeaponRequest>,
    cancelling: bool,
}

struct EquipmentHandoff {
    equipment: Box<dyn EquipmentWeaponHandoff>,
}

impl SourceWeaponHandoff for EquipmentHandoff {
    fn provider(&self) -> ProviderId {
        self.equipment.weapon().provider.clone()
    }
    fn accepts(&self, item: &ItemId) -> bool {
        *item == self.equipment.weapon().item
    }
    fn select(&mut self, item: &ItemId) -> bool {
        *item == self.equipment.weapon().item
    }
    fn holster(&mut self) {
        self.equipment.holster();
    }
    fn is_holstered(&self) -> bool {
        self.equipment.is_holstered()
    }
    fn resume(&mut self, _item: Option<&ItemId>) -> ResumeOutcome {
        self.equipment.resume();
        ResumeOutcome::Immediate(true)
    }
    fn restore_request(&mut self, _id: u64, _item: Option<&ItemId>) -> Box<dyn SourceWeaponRequest> {
        Box::new(RefusedRequest)
    }
    fn is_deferred(&self) -> bool {
        false
    }
}

/// Fail-closed refused request; immediate handoffs never take the restore-request path.
struct RefusedRequest;

impl SourceWeaponRequest for RefusedRequest {
    fn id(&self) -> u64 {
        u64::MAX
    }
    fn status(&mut self) -> RequestStatus {
        RequestStatus::Refused
    }
    fn cancel(&mut self) {}
}

/// One actor's source selection.
pub struct WeaponSlot {
    current: WeaponSlotState,
    restore_pending: bool,
    activation: Option<Activation>,
    bindings: HashMap<ProviderId, Binding>,
    primary_provider: ProviderId,
    equipment_provider: Option<ProviderId>,
    next_generation: u64,
    actor_current: Box<dyn Fn() -> bool>,
}

impl WeaponSlot {
    /// Build a slot; legacy restores lower once bindings are available.
    pub fn new(
        primary: Box<dyn SourceWeaponHandoff>,
        equipment: Option<Box<dyn EquipmentWeaponHandoff>>,
        restored: Option<WeaponSlotRestoreState>,
        actor_current: Box<dyn Fn() -> bool>,
    ) -> Result<Self, WeaponSlotError> {
        let primary_provider = primary.provider();
        let mut slot = Self {
            current: WeaponSlotState::Active {
                provider: primary_provider.clone(),
            },
            restore_pending: restored.is_some(),
            activation: None,
            bindings: HashMap::new(),
            primary_provider: primary_provider.clone(),
            equipment_provider: None,
            next_generation: 0,
            actor_current,
        };
        slot.insert_binding(primary, Rc::new(Cell::new(true)), None);
        if let Some(equipment) = equipment {
            let provider = equipment.weapon().provider.clone();
            if provider == primary_provider {
                return Err(WeaponSlotError::OwnerOverlap);
            }
            slot.equipment_provider = Some(provider);
            slot.insert_binding(Box::new(EquipmentHandoff { equipment }), Rc::new(Cell::new(true)), None);
        }
        let restored = restored.unwrap_or_else(|| {
            WeaponSlotRestoreState::State(WeaponSlotState::Active {
                provider: primary_provider.clone(),
            })
        });
        slot.current = slot.restore_state(&restored)?;
        Ok(slot)
    }

    fn insert_binding(
        &mut self,
        handoff: Box<dyn SourceWeaponHandoff>,
        live: Rc<Cell<bool>>,
        read: Option<WeaponReadFn>,
    ) -> u64 {
        let generation = self.next_generation;
        self.next_generation += 1;
        self.bindings.insert(
            handoff.provider(),
            Binding {
                handoff,
                live,
                read,
                generation,
            },
        );
        generation
    }

    fn restore_state(&self, state: &WeaponSlotRestoreState) -> Result<WeaponSlotState, WeaponSlotError> {
        match state {
            WeaponSlotRestoreState::State(state) => Ok(state.clone()),
            WeaponSlotRestoreState::Primary => Ok(WeaponSlotState::Active {
                provider: self.primary_provider.clone(),
            }),
            WeaponSlotRestoreState::Equipment => self
                .equipment_provider
                .clone()
                .map(|provider| WeaponSlotState::Active { provider })
                .ok_or(WeaponSlotError::EquipmentWithoutOwner),
            WeaponSlotRestoreState::HolsteringPrimary { next } => Ok(WeaponSlotState::Switching {
                from: self.primary_provider.clone(),
                next: next.clone(),
            }),
            WeaponSlotRestoreState::HolsteringEquipment { next } => self
                .equipment_provider
                .clone()
                .map(|from| WeaponSlotState::Switching {
                    from,
                    next: next.clone(),
                })
                .ok_or(WeaponSlotError::EquipmentTransitionWithoutOwner),
        }
    }

    /// Snapshot the live state.
    #[must_use]
    pub fn snapshot(&self) -> WeaponSlotState {
        self.current.clone()
    }

    /// Validate a restored state against live bindings; runs once.
    pub fn validate_restore(&mut self) -> Result<(), WeaponSlotError> {
        if !self.restore_pending {
            return Ok(());
        }
        let state = self.current.clone();
        let outgoing = match &state {
            WeaponSlotState::Active { provider } => provider.clone(),
            WeaponSlotState::Switching { from, .. } | WeaponSlotState::Activating { from, .. } => from.clone(),
        };
        let outgoing_generation = self
            .binding_generation(&outgoing)
            .ok_or(WeaponSlotError::RestoreUnbound)?;
        if let Some(read) = self.bindings.get(&outgoing).and_then(|binding| binding.read.as_ref()) {
            let presentation = read();
            let declared = presentation
                .active
                .as_ref()
                .is_none_or(|active| presentation.items.iter().any(|item| item.item == *active));
            if presentation.source_provider != outgoing
                || !declared
                || self.binding_generation(&outgoing) != Some(outgoing_generation)
            {
                return Err(WeaponSlotError::RestoreUndeclared);
            }
        }
        let (next, request) = match &state {
            WeaponSlotState::Active { .. } => (None, None),
            WeaponSlotState::Switching { next, .. } => (Some(next), None),
            WeaponSlotState::Activating { next, request, .. } => (Some(next), Some(*request)),
        };
        if let Some(next) = next {
            let incoming = self
                .binding_generation(&next.provider)
                .ok_or(WeaponSlotError::RestoreUnadmitted)?;
            let accepts = self
                .bindings
                .get(&next.provider)
                .is_some_and(|binding| binding.handoff.accepts(&next.item));
            if !accepts || self.binding_generation(&next.provider) != Some(incoming) {
                return Err(WeaponSlotError::RestoreUnadmitted);
            }
            if let Some(id) = request {
                let deferred = self
                    .bindings
                    .get(&next.provider)
                    .is_some_and(|binding| binding.handoff.is_deferred());
                if !deferred {
                    return Err(WeaponSlotError::RestoreNeedsInputOwner);
                }
                let request = self
                    .bindings
                    .get_mut(&next.provider)
                    .map(|binding| binding.handoff.restore_request(id, Some(&next.item)))
                    .ok_or(WeaponSlotError::RestoreUnadmitted)?;
                if request.id() != id
                    || self.current != state
                    || self.binding_generation(&next.provider) != Some(incoming)
                {
                    return Err(WeaponSlotError::RestoreChanged);
                }
                self.activation = Some(Activation {
                    state: state.clone(),
                    request,
                    cancelling: false,
                });
            }
        }
        self.restore_pending = false;
        Ok(())
    }

    /// Whether the provider is the active selection.
    #[must_use]
    pub fn selected(&self, provider: &ProviderId) -> bool {
        (self.actor_current)()
            && matches!(&self.current, WeaponSlotState::Active { provider: active } if active == provider)
            && self.binding_generation(provider).is_some()
    }

    /// Whether the provider is presented (active or holstering out).
    #[must_use]
    pub fn presented(&self, provider: &ProviderId) -> bool {
        let shown = match &self.current {
            WeaponSlotState::Active { provider } => provider,
            WeaponSlotState::Switching { from, .. } | WeaponSlotState::Activating { from, .. } => from,
        };
        (self.actor_current)() && shown == provider && self.binding_generation(provider).is_some()
    }

    /// Whether the primary source is selected.
    #[must_use]
    pub fn primary_selected(&self) -> bool {
        self.selected(&self.primary_provider.clone())
    }

    /// Whether the equipment source is selected.
    #[must_use]
    pub fn equipment_selected(&self) -> bool {
        self.equipment_provider
            .as_ref()
            .is_some_and(|provider| self.selected(provider))
    }

    fn binding_generation(&self, provider: &ProviderId) -> Option<u64> {
        let binding = self.bindings.get(provider)?;
        if !(self.actor_current)() || !binding.live.get() {
            return None;
        }
        Some(binding.generation)
    }

    /// Bind another source owner; unbind with the returned token.
    pub fn bind(
        &mut self,
        handoff: Box<dyn SourceWeaponHandoff>,
        live: Rc<Cell<bool>>,
        read: Option<WeaponReadFn>,
    ) -> Result<BindingToken, WeaponSlotError> {
        let provider = handoff.provider();
        if !(self.actor_current)() || !live.get() || self.bindings.contains_key(&provider) {
            return Err(WeaponSlotError::BindUnavailable);
        }
        let generation = self.insert_binding(handoff, live, read);
        Ok(BindingToken { provider, generation })
    }

    /// Release a binding; stale tokens are a no-op.
    pub fn unbind(&mut self, token: &BindingToken) -> Result<(), WeaponSlotError> {
        let current = self.bindings.get(&token.provider).map(|binding| binding.generation);
        if current != Some(token.generation) {
            return Ok(());
        }
        let provider = token.provider.clone();
        self.bindings.remove(&provider);
        if !(self.actor_current)() || self.restore_pending {
            return Ok(());
        }
        let state = self.current.clone();
        match &state {
            WeaponSlotState::Active { provider: active } if *active == provider => {
                self.cancel_activation(&state);
                self.fallback()?;
            }
            WeaponSlotState::Switching { from, .. } | WeaponSlotState::Activating { from, .. } if *from == provider => {
                self.cancel_activation(&state);
                self.fallback()?;
            }
            WeaponSlotState::Switching { from, next } | WeaponSlotState::Activating { from, next, .. }
                if next.provider == provider =>
            {
                self.cancel_activation(&state);
                if self.current == state {
                    let from = from.clone();
                    self.resume_slot(&from, None, None)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Read every live bound presentation.
    pub fn presentations(&self) -> Result<Vec<SourceWeaponPresentation>, WeaponSlotError> {
        if !(self.actor_current)() {
            return Ok(Vec::new());
        }
        let mut result = Vec::new();
        for (provider, entry) in &self.bindings {
            let Some(read) = entry.read.as_ref() else { continue };
            if self.binding_generation(provider) != Some(entry.generation) {
                continue;
            }
            let state = read();
            if state.source_provider != *provider {
                return Err(WeaponSlotError::ForeignPresentation);
            }
            if self.binding_generation(provider) == Some(entry.generation) {
                result.push(state);
            }
        }
        Ok(result)
    }

    /// Request a weapon selection.
    pub fn request(&mut self, weapon: &WeaponReference) -> Result<bool, WeaponSlotError> {
        self.validate_restore()?;
        if !(self.actor_current)() {
            return Ok(false);
        }
        let destination = match self.binding_generation(&weapon.provider) {
            Some(generation) => generation,
            None => return Ok(false),
        };
        let accepts = self
            .bindings
            .get(&weapon.provider)
            .is_some_and(|binding| binding.handoff.accepts(&weapon.item));
        if !accepts || self.binding_generation(&weapon.provider) != Some(destination) {
            return Ok(false);
        }
        if matches!(self.current, WeaponSlotState::Activating { .. }) {
            let state = self.current.clone();
            self.reconcile_activation(&state)?;
        }
        let state = self.current.clone();
        match &state {
            WeaponSlotState::Activating { from, next, .. } => {
                if next.provider == weapon.provider && next.item == weapon.item {
                    return Ok(true);
                }
                self.cancel_activation(&state);
                if !(self.actor_current)()
                    || self.current != state
                    || self.binding_generation(&weapon.provider) != Some(destination)
                {
                    return Ok(false);
                }
                if weapon.provider == *from {
                    self.resume_slot(from, Some(weapon.item.clone()), None)?;
                } else {
                    self.current = WeaponSlotState::Switching {
                        from: from.clone(),
                        next: weapon.clone(),
                    };
                }
                Ok(true)
            }
            WeaponSlotState::Switching { from, .. } => {
                self.current = WeaponSlotState::Switching {
                    from: from.clone(),
                    next: weapon.clone(),
                };
                Ok(true)
            }
            WeaponSlotState::Active { provider } => {
                if *provider == weapon.provider {
                    let accepted = self
                        .bindings
                        .get_mut(provider)
                        .is_some_and(|binding| binding.handoff.select(&weapon.item));
                    Ok(accepted
                        && (self.actor_current)()
                        && self.binding_generation(&weapon.provider) == Some(destination))
                } else {
                    if self.binding_generation(provider).is_none() {
                        self.fallback()?;
                        return Ok(false);
                    }
                    self.current = WeaponSlotState::Switching {
                        from: provider.clone(),
                        next: weapon.clone(),
                    };
                    if let Some(binding) = self.bindings.get_mut(provider) {
                        binding.handoff.holster();
                    }
                    Ok(true)
                }
            }
        }
    }

    fn fallback(&mut self) -> Result<(), WeaponSlotError> {
        let primary = self.primary_provider.clone();
        self.resume_slot(&primary, None, None)
    }

    fn cancel_activation(&mut self, state: &WeaponSlotState) {
        let matches =
            matches!(&self.activation, Some(activation) if activation.state == *state && !activation.cancelling);
        if !matches {
            return;
        }
        if let Some(activation) = self.activation.as_mut() {
            activation.cancelling = true;
            activation.request.cancel();
        }
        self.activation = None;
    }

    fn resume_slot(
        &mut self,
        provider: &ProviderId,
        item: Option<ItemId>,
        outgoing: Option<(ProviderId, u64)>,
    ) -> Result<(), WeaponSlotError> {
        let generation = self.binding_generation(provider).ok_or(WeaponSlotError::NoLiveOwner)?;
        let activated = WeaponSlotState::Active {
            provider: provider.clone(),
        };
        self.current = activated.clone();
        let outcome = self
            .bindings
            .get_mut(provider)
            .map(|binding| binding.handoff.resume(item.as_ref()))
            .ok_or(WeaponSlotError::NoLiveOwner)?;
        let accepted = match outcome {
            ResumeOutcome::Immediate(accepted) => accepted,
            ResumeOutcome::Deferred(mut request) => {
                if !(self.actor_current)()
                    || self.current != activated
                    || self.binding_generation(provider) != Some(generation)
                {
                    request.cancel();
                    return Ok(());
                }
                let status = request.status();
                if !(self.actor_current)()
                    || self.current != activated
                    || self.binding_generation(provider) != Some(generation)
                {
                    request.cancel();
                    return Ok(());
                }
                if status == RequestStatus::Pending {
                    let held = match (&item, &outgoing) {
                        (Some(item), Some((outgoing, generation)))
                            if self.binding_generation(outgoing) == Some(*generation) =>
                        {
                            Some((outgoing.clone(), item.clone()))
                        }
                        _ => None,
                    };
                    let Some((outgoing, item)) = held else {
                        request.cancel();
                        return Err(WeaponSlotError::ResumeWithoutOutgoing);
                    };
                    let state = WeaponSlotState::Activating {
                        from: outgoing,
                        next: WeaponReference {
                            provider: provider.clone(),
                            item,
                        },
                        request: request.id(),
                    };
                    self.current = state.clone();
                    self.activation = Some(Activation {
                        state,
                        request,
                        cancelling: false,
                    });
                    return Ok(());
                }
                status == RequestStatus::Accepted
            }
        };
        if accepted
            || !(self.actor_current)()
            || self.current != activated
            || self.binding_generation(provider) != Some(generation)
        {
            return Ok(());
        }
        if let Some((outgoing, generation)) = &outgoing {
            if self.binding_generation(outgoing) == Some(*generation) {
                let outgoing = outgoing.clone();
                return self.resume_slot(&outgoing, None, None);
            }
        }
        Err(WeaponSlotError::ResumeRefused)
    }

    fn reconcile_activation(&mut self, state: &WeaponSlotState) -> Result<(), WeaponSlotError> {
        let (from, next) = match state {
            WeaponSlotState::Activating { from, next, .. } => (from.clone(), next.clone()),
            _ => return Err(WeaponSlotError::ActivationLost),
        };
        if !matches!(&self.activation, Some(activation) if activation.state == *state) {
            return Err(WeaponSlotError::ActivationLost);
        }
        if matches!(&self.activation, Some(activation) if activation.cancelling) {
            return Ok(());
        }
        let outgoing = self.binding_generation(&from);
        let incoming = self.binding_generation(&next.provider);
        if outgoing.is_none() {
            self.cancel_activation(state);
            if self.current == *state {
                self.fallback()?;
            }
            return Ok(());
        }
        if incoming.is_none() {
            self.cancel_activation(state);
            if self.current == *state {
                self.resume_slot(&from, None, None)?;
            }
            return Ok(());
        }
        let status = self
            .activation
            .as_mut()
            .map(|activation| activation.request.status())
            .unwrap_or(RequestStatus::Refused);
        if !(self.actor_current)()
            || self.current != *state
            || !matches!(&self.activation, Some(activation) if activation.state == *state)
            || self.binding_generation(&from) != outgoing
            || self.binding_generation(&next.provider) != incoming
        {
            return Ok(());
        }
        if status == RequestStatus::Pending {
            return Ok(());
        }
        self.activation = None;
        if status == RequestStatus::Accepted {
            self.current = WeaponSlotState::Active {
                provider: next.provider.clone(),
            };
        } else {
            self.resume_slot(&from, None, None)?;
        }
        Ok(())
    }

    /// Reconcile after source traversal; never advances a source animation.
    pub fn reconcile(&mut self) -> Result<(), WeaponSlotError> {
        self.validate_restore()?;
        if !(self.actor_current)() {
            return Ok(());
        }
        let state = self.current.clone();
        match &state {
            WeaponSlotState::Activating { .. } => self.reconcile_activation(&state),
            WeaponSlotState::Active { provider } => {
                if self.binding_generation(provider).is_none() {
                    self.fallback()?;
                }
                Ok(())
            }
            WeaponSlotState::Switching { from, next } => {
                let outgoing = match self.binding_generation(from) {
                    Some(generation) => generation,
                    None => {
                        self.fallback()?;
                        return Ok(());
                    }
                };
                let incoming = self.binding_generation(&next.provider);
                let accepted = incoming.is_some_and(|_| {
                    self.bindings
                        .get(&next.provider)
                        .is_some_and(|binding| binding.handoff.accepts(&next.item))
                });
                if !(self.actor_current)() || self.current != state {
                    return Ok(());
                }
                let incoming_now = self.binding_generation(&next.provider);
                if !accepted || incoming.is_none() || incoming_now != incoming {
                    self.resume_slot(from, None, None)?;
                    return Ok(());
                }
                let holstered = self
                    .bindings
                    .get_mut(from)
                    .is_some_and(|binding| binding.handoff.is_holstered());
                if holstered
                    && self.current == state
                    && self.binding_generation(from) == Some(outgoing)
                    && self.binding_generation(&next.provider) == incoming_now
                {
                    self.resume_slot(&next.provider, Some(next.item.clone()), Some((from.clone(), outgoing)))?;
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn provider(name: &str) -> ProviderId {
        ProviderId {
            namespace: "test".to_string(),
            name: name.to_string(),
        }
    }

    #[derive(Clone)]
    struct FakeState {
        holstered: Rc<Cell<bool>>,
        selected: Rc<RefCell<Option<ItemId>>>,
    }

    struct ImmediateFake {
        provider: ProviderId,
        items: Vec<ItemId>,
        state: FakeState,
    }

    impl SourceWeaponHandoff for ImmediateFake {
        fn provider(&self) -> ProviderId {
            self.provider.clone()
        }
        fn accepts(&self, item: &ItemId) -> bool {
            self.items.contains(item)
        }
        fn select(&mut self, item: &ItemId) -> bool {
            if !self.items.contains(item) {
                return false;
            }
            *self.state.selected.borrow_mut() = Some(item.clone());
            true
        }
        fn holster(&mut self) {
            self.state.holstered.set(true);
        }
        fn is_holstered(&self) -> bool {
            self.state.holstered.get()
        }
        fn resume(&mut self, item: Option<&ItemId>) -> ResumeOutcome {
            self.state.holstered.set(false);
            if let Some(item) = item {
                *self.state.selected.borrow_mut() = Some(item.clone());
            }
            ResumeOutcome::Immediate(true)
        }
        fn restore_request(&mut self, _id: u64, _item: Option<&ItemId>) -> Box<dyn SourceWeaponRequest> {
            Box::new(RefusedRequest)
        }
        fn is_deferred(&self) -> bool {
            false
        }
    }

    fn fake(name: &str, items: &[&str]) -> (Box<dyn SourceWeaponHandoff>, FakeState) {
        let state = FakeState {
            holstered: Rc::new(Cell::new(true)),
            selected: Rc::new(RefCell::new(None)),
        };
        let fake = ImmediateFake {
            provider: provider(name),
            items: items.iter().map(|item| item.to_string()).collect(),
            state: state.clone(),
        };
        (Box::new(fake), state)
    }

    struct EquipmentFake {
        weapon: WeaponReference,
        holstered: Rc<Cell<bool>>,
    }

    impl EquipmentWeaponHandoff for EquipmentFake {
        fn weapon(&self) -> &WeaponReference {
            &self.weapon
        }
        fn holster(&mut self) {
            self.holstered.set(true);
        }
        fn is_holstered(&self) -> bool {
            self.holstered.get()
        }
        fn resume(&mut self) {
            self.holstered.set(false);
        }
    }

    fn equipment(name: &str, item: &str) -> Box<dyn EquipmentWeaponHandoff> {
        Box::new(EquipmentFake {
            weapon: WeaponReference {
                provider: provider(name),
                item: item.to_string(),
            },
            holstered: Rc::new(Cell::new(false)),
        })
    }

    fn live_actor() -> Box<dyn Fn() -> bool> {
        Box::new(|| true)
    }

    #[test]
    fn defaults_to_active_primary() {
        let (primary, _) = fake("primary", &["q1:axe"]);
        let slot = WeaponSlot::new(primary, None, None, live_actor()).unwrap();
        assert_eq!(
            slot.snapshot(),
            WeaponSlotState::Active {
                provider: provider("primary")
            }
        );
        assert!(slot.primary_selected());
        assert!(slot.presented(&provider("primary")));
    }

    #[test]
    fn rejects_overlapping_equipment() {
        let (primary, _) = fake("primary", &["q1:axe"]);
        let err = WeaponSlot::new(primary, Some(equipment("primary", "q1:grenade")), None, live_actor())
            .err()
            .unwrap();
        assert_eq!(err, WeaponSlotError::OwnerOverlap);
    }

    #[test]
    fn selects_within_active_source() {
        let (primary, state) = fake("primary", &["q1:axe", "q1:shotgun"]);
        let mut slot = WeaponSlot::new(primary, None, None, live_actor()).unwrap();
        let weapon = WeaponReference {
            provider: provider("primary"),
            item: "q1:shotgun".to_string(),
        };
        assert!(slot.request(&weapon).unwrap());
        assert_eq!(*state.selected.borrow(), Some("q1:shotgun".to_string()));
        assert!(slot.primary_selected());
    }

    #[test]
    fn switches_sources_through_holster() {
        let (primary, _) = fake("primary", &["q1:axe"]);
        let (other, _) = fake("other", &["q2:blaster"]);
        let mut slot = WeaponSlot::new(primary, None, None, live_actor()).unwrap();
        let live = Rc::new(Cell::new(true));
        slot.bind(other, live, None).unwrap();
        let weapon = WeaponReference {
            provider: provider("other"),
            item: "q2:blaster".to_string(),
        };
        assert!(slot.request(&weapon).unwrap());
        assert!(matches!(slot.snapshot(), WeaponSlotState::Switching { .. }));
        slot.reconcile().unwrap();
        assert!(slot.selected(&provider("other")));
    }

    #[test]
    fn restores_saved_equipment_selection() {
        let (primary, _) = fake("primary", &["q1:axe"]);
        let mut slot = WeaponSlot::new(
            primary,
            Some(equipment("equip", "q1:grenade")),
            Some(WeaponSlotRestoreState::Equipment),
            live_actor(),
        )
        .unwrap();
        slot.reconcile().unwrap();
        assert!(slot.equipment_selected());
    }

    #[test]
    fn rejects_duplicate_bind_and_ignores_stale_unbind() {
        let (primary, _) = fake("primary", &["q1:axe"]);
        let (other, _) = fake("other", &["q2:blaster"]);
        let mut slot = WeaponSlot::new(primary, None, None, live_actor()).unwrap();
        let token = slot.bind(other, Rc::new(Cell::new(true)), None).unwrap();
        let (dupe, _) = fake("other", &["q2:blaster"]);
        assert_eq!(
            slot.bind(dupe, Rc::new(Cell::new(true)), None).unwrap_err(),
            WeaponSlotError::BindUnavailable
        );
        slot.unbind(&token).unwrap();
        slot.unbind(&token).unwrap();
        assert!(slot.primary_selected());
    }
}
