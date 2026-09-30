//! id1 pickup bindings (`src/content/q1/quakec/id1-pickups.ts`).
//!
//! Donor provenance: `src/content/q1/quakec/id1-pickups.ts`
//! (`Id1PickupBinding`, `QcPickupPolicy`).

use std::cell::RefCell;

use qa_core::identity::OwnedActor;
use qa_core::numeric::float_to_wrapped_i32;
use qa_core::time::SourceTime;

use crate::contract::{
    ItemId, OriginalPickupAdmission, OriginalPickupGrant, OriginalPickupOffer, OriginalPickupOutcome, PickupAmmoGrant,
    PickupCargoEntry, PickupCargoKind, PickupCount, PickupSupplyOffer, PickupWeaponGrant, SourcePickupLifetime,
    SourcePickupSelection,
};

use super::pickup_stage::{
    qc_pickup_stages, QcPickupDescriptor, QcPickupOperation, QcPickupScalar, QcPickupStage, QcPickupStageDescriptor,
    QcPickupValue,
};
use super::qc_view::{
    MachineFn, QcCallSite, QcEntityStoreObservation, QcFunctionBoundary, QcFunctionExecution, QcHostSource,
    QcInlineBoundary, QcInlineContinuation, QcInlineRegion, QcMachineView,
};
use super::{fround, FrameGuard, QcError};

/// Selected-weapon policy (donor `QcPickupPolicy`).
pub trait QcPickupPolicy {
    /// Currently selected weapon.
    fn current(&self, actor: &OwnedActor) -> Option<ItemId>;
    /// Select a weapon.
    fn select(&self, actor: &OwnedActor, item: &ItemId) -> Result<(), QcError>;
    /// Project a counter through the original region.
    fn counter(
        &self,
        actor: &OwnedActor,
        item: &ItemId,
        original: &mut dyn FnMut(f64) -> Result<Option<f64>, QcError>,
    ) -> Result<(), QcError>;
}

/// Policy accessor (donor `policy?: () => QcPickupPolicy | null`).
pub type PolicyFn<'a> = Box<dyn Fn() -> Option<&'a dyn QcPickupPolicy> + 'a>;

/// Primary-weapon selection predicate (donor `primaryWeaponSelected`).
pub type PrimaryWeaponSelectedFn<'a> = Box<dyn Fn(&OwnedActor) -> bool + 'a>;

/// Weapon ownership predicate (donor `ownsWeapon`).
pub type OwnsWeaponFn<'a> = Box<dyn Fn(&OwnedActor, &ItemId) -> bool + 'a>;

/// Source actor with its slot.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceActor {
    actor: OwnedActor,
    slot: usize,
}

/// Active pickup call (donor `PickupCall`).
struct PickupCall {
    stage: QcPickupStage,
    descriptor: QcPickupDescriptor,
    pickup: SourceActor,
    recipient: SourceActor,
    selection: SourcePickupSelection<'static>,
    cancel_owner: u64,
    lifetime: &'static dyn SourcePickupLifetime,
    new_weapon: bool,
    source_effect: bool,
    accepted: bool,
    consuming: bool,
    consumed: bool,
}

/// Counter projection capture.
struct PickupProjection {
    region: QcInlineRegion,
    reference: i32,
    word: usize,
    value: Option<f64>,
}

/// Selected supply (donor `supply` result).
#[derive(Debug, Clone, PartialEq)]
pub struct QcPickupSupplyOffer {
    /// Supply offer.
    pub offer: PickupSupplyOffer,
    /// Whether the source leaves the supply behind.
    pub leave: bool,
}

/// Extend a run-source selection borrow to the frame lifetime.
///
/// # Safety
///
/// Sound only when the handle is stored in a frame that is popped via
/// [`FrameGuard`] before the lending `run_source` call returns, on all
/// paths including panic unwind. See the module note on
/// [`Id1PickupBinding`] for the full contract argument.
unsafe fn extend_selection<'x>(selection: SourcePickupSelection<'x>) -> SourcePickupSelection<'static> {
    // Lifetimes are erased at runtime; the value layout is identical.
    unsafe { std::mem::transmute::<SourcePickupSelection<'x>, SourcePickupSelection<'static>>(selection) }
}

/// Extend a run-source lifetime borrow to the frame lifetime.
///
/// # Safety
///
/// Same contract as [`extend_selection`].
unsafe fn extend_lifetime<'y>(lifetime: &'y dyn SourcePickupLifetime) -> &'static dyn SourcePickupLifetime {
    // Fat pointers share their layout across borrow lengths.
    unsafe { std::mem::transmute::<&'y dyn SourcePickupLifetime, &'static dyn SourcePickupLifetime>(lifetime) }
}

/// Extend a region continuation borrow to a `'static` remove callback.
///
/// # Safety
///
/// Sound only when the [`SourcePickupLifetime`] implementation invokes
/// (and drops) the callback synchronously before `consume_pickup`
/// returns, matching the donor implementation
/// (`world/gameplay/original-pickups.ts` calls `remove()` between its
/// scope checks and never stores it). A `consume_pickup` that retains
/// the callback past its return would resume a dead VM continuation.
unsafe fn extend_execute<'x>(execute: &'x dyn QcInlineContinuation) -> &'static dyn QcInlineContinuation {
    // Fat pointers share their layout across borrow lengths.
    unsafe { std::mem::transmute::<&'x dyn QcInlineContinuation, &'static dyn QcInlineContinuation>(execute) }
}

/// Holds the selected resource owner through the complete original
/// touch and target continuation (donor `Id1PickupBinding`).
///
/// # Scoped selection and lifetime handles
///
/// `OriginalPickupAdmission::run_source` lends its selection and
/// lifetime with higher-ranked lifetimes, so they cannot be named in
/// field types even though the donor stores them in the active frame.
/// This binding extends those borrows for frame storage
/// ([`extend_selection`], [`extend_lifetime`]), which is sound because
/// every stored handle is popped by a [`FrameGuard`] before the
/// lending `run_source` call returns, on all paths including panic
/// unwind, and frames never escape the callback. If the contract
/// trait ever lends owned handles instead, the `unsafe` blocks and
/// this note collapse back to plain storage.
///
/// The admission is a generic parameter because
/// `OriginalPickupAdmission` has a generic method and is not
/// `dyn`-compatible.
pub struct Id1PickupBinding<'a, A: OriginalPickupAdmission> {
    source: QcHostSource<'a>,
    admission: &'a A,
    machine: MachineFn<'a>,
    primary_weapon_selected: PrimaryWeaponSelectedFn<'a>,
    owns_weapon: Option<OwnsWeaponFn<'a>>,
    policy: Option<PolicyFn<'a>>,
    stages: Vec<QcPickupStage>,
    active: RefCell<Vec<Option<PickupCall>>>,
    projection: RefCell<Option<PickupProjection>>,
}

impl<'a, A: OriginalPickupAdmission> Id1PickupBinding<'a, A> {
    /// Bind original pickup callers (donor `Id1PickupBinding`
    /// constructor).
    pub fn new(
        source: QcHostSource<'a>,
        admission: &'a A,
        machine: MachineFn<'a>,
        primary_weapon_selected: Option<PrimaryWeaponSelectedFn<'a>>,
        owns_weapon: Option<OwnsWeaponFn<'a>>,
        policy: Option<PolicyFn<'a>>,
        declared: Vec<QcPickupStage>,
    ) -> Result<Self, QcError> {
        let replacement: std::collections::HashSet<usize> = declared.iter().map(|stage| stage.function_index).collect();
        let mut stages: Vec<QcPickupStage> = qc_pickup_stages(source.program)?
            .into_iter()
            .filter(|stage| !replacement.contains(&stage.function_index))
            .collect();
        stages.extend(declared);
        Ok(Self {
            source,
            admission,
            machine,
            primary_weapon_selected: primary_weapon_selected.unwrap_or_else(|| Box::new(|_| true)),
            owns_weapon,
            policy,
            stages,
            active: RefCell::new(Vec::new()),
            projection: RefCell::new(None),
        })
    }

    /// Fail when a pickup caller is active (donor `assertIdle`).
    pub fn assert_idle(&self) -> Result<(), QcError> {
        if !self.active.borrow().is_empty() {
            return Err(QcError::program(
                "Cannot save during an original pickup caller",
                self.source.program.source,
            ));
        }
        Ok(())
    }

    /// Observe an entity store (donor `observeStore`).
    pub fn observe_store(&self, store: &QcEntityStoreObservation) -> Result<(), QcError> {
        let mut projection = self.projection.borrow_mut();
        let Some(projected) = projection.as_mut() else {
            return Ok(());
        };
        if store.reference != projected.reference
            || store.word != projected.word
            || store.after.len() != 4
            || store.function_index != projected.region.function_index
            || store.statement < projected.region.entry
            || store.statement >= projected.region.exit
        {
            return Err(QcError::program(
                "Original pickup counter region wrote outside its qualified field",
                self.source.program.source,
            ));
        }
        if projected.value.is_some() {
            return Err(QcError::program(
                "Original pickup counter region stored more than once",
                self.source.program.source,
            ));
        }
        let bytes: [u8; 4] = store.after.as_slice().try_into().map_err(|_| {
            QcError::program(
                "Original pickup counter region wrote outside its qualified field",
                self.source.program.source,
            )
        })?;
        projected.value = Some(f64::from(f32::from_le_bytes(bytes)));
        Ok(())
    }

    /// Whether selection is deferred to the source (donor
    /// `selectionDeferred`).
    pub fn selection_deferred(&self, offer: &OriginalPickupOffer) -> Result<bool, QcError> {
        let active = self.active.borrow();
        let frame = active.last();
        let Some(Some(frame)) = frame else {
            return Err(QcError::program(
                "Pickup selection requires its held source caller",
                self.source.program.source,
            ));
        };
        if frame.pickup.actor.id() != &offer.pickup || frame.recipient.actor.id() != &offer.recipient {
            return Err(QcError::program(
                "Pickup selection requires its held source caller",
                self.source.program.source,
            ));
        }
        let deferred = frame.stage.source_selection.is_some();
        drop(active);
        self.validate()?;
        Ok(deferred)
    }

    /// Selected supply (donor `supply`).
    pub fn supply(&self, offer: &OriginalPickupOffer) -> Result<QcPickupSupplyOffer, QcError> {
        let active = self.active.borrow();
        let Some(Some(frame)) = active.last() else {
            return Err(QcError::program(
                "Selected supply requires its current original pickup caller",
                self.source.program.source,
            ));
        };
        if frame.pickup.actor.id() != &offer.pickup
            || frame.recipient.actor.id() != &offer.recipient
            || frame.descriptor.item != offer.item
            || frame.descriptor.supply.is_none()
        {
            return Err(QcError::program(
                "Selected supply requires its current original pickup caller",
                self.source.program.source,
            ));
        }
        let descriptor = frame.descriptor.clone();
        let pickup_slot = frame.pickup.slot;
        drop(active);
        self.validate()?;
        let vm = self.vm()?;
        let supply = descriptor.supply.as_ref().ok_or_else(|| {
            QcError::program(
                "Selected supply requires its current original pickup caller",
                self.source.program.source,
            )
        })?;
        let amount = self.scalar(&supply.quantity, pickup_slot)?;
        let mut leave = false;
        if let Some(word) = supply.leave {
            leave = vm.global_float(word as usize)? != 0.0;
        }
        Ok(QcPickupSupplyOffer {
            offer: match supply.leave {
                None => PickupSupplyOffer::Ammo(PickupAmmoGrant {
                    item: supply.item.clone(),
                    amount,
                }),
                Some(_) => PickupSupplyOffer::Weapon(PickupWeaponGrant {
                    item: descriptor.item.clone(),
                    ammo: vec![PickupAmmoGrant {
                        item: supply.item.clone(),
                        amount,
                    }],
                }),
            },
            leave,
        })
    }

    /// Machine accessor with ownership check (donor `vm()`).
    fn vm(&self) -> Result<&'a dyn QcMachineView, QcError> {
        let vm = (self.machine)();
        if vm.program_digest() != self.source.program.digest {
            return Err(QcError::program(
                "Original pickups belong to another machine",
                self.source.program.source,
            ));
        }
        Ok(vm)
    }

    /// Read a scalar source (donor `scalar`).
    fn scalar(&self, input: &QcPickupScalar, slot: usize) -> Result<f64, QcError> {
        let vm = self.vm()?;
        match input {
            QcPickupScalar::Field { name } => vm.entity_float(slot, vm.field_offset(name)?),
            QcPickupScalar::Global { word } => vm.global_float(*word as usize),
        }
    }

    /// Actor at a reference (donor `actor`).
    fn actor(&self, reference: i32) -> Result<Option<SourceActor>, QcError> {
        let vm = self.vm()?;
        let slot = vm.entity_slot(reference)?;
        match self.source.slots.at(slot) {
            Some(actor) if !self.source.slots.is_free(slot) => Ok(Some(SourceActor { actor, slot })),
            _ => Ok(None),
        }
    }

    /// Whether a source actor is still bound (donor `live`).
    fn live(&self, value: &SourceActor) -> bool {
        self.source.actors.resolve_owned(value.actor.id()).as_ref() == Some(&value.actor)
            && self.source.slots.at(value.slot).as_ref() == Some(&value.actor)
            && !self.source.slots.is_free(value.slot)
    }

    /// Called before source calls and entity accesses, including those
    /// made by target functions (donor `validate`).
    pub fn validate(&self) -> Result<(), QcError> {
        let stale = {
            let active = self.active.borrow();
            let mut stale: Option<u64> = None;
            for frame in active.iter().flatten() {
                let replacement_current = match &frame.selection {
                    SourcePickupSelection::Replacement { current, .. } => Some(current()),
                    _ => None,
                };
                if (!frame.consuming && !frame.consumed && !self.live(&frame.pickup))
                    || !self.live(&frame.recipient)
                    || !frame.consuming
                        && matches!(frame.selection, SourcePickupSelection::Replacement { .. })
                        && replacement_current == Some(false)
                {
                    stale = Some(frame.cancel_owner);
                    break;
                }
            }
            stale
        };
        if let Some(owner) = stale {
            return Err(QcError::cancelled(owner, [0, 0, 0]));
        }
        Ok(())
    }

    /// Compose function boundaries (donor `composeFunctions`).
    pub fn compose_functions(&'a self, inner: QcFunctionBoundary<'a>) -> Result<QcFunctionBoundary<'a>, QcError> {
        if self
            .stages
            .iter()
            .any(|stage| inner.functions.contains(&stage.function_index))
        {
            return Err(QcError::program(
                "Original pickup caller already has a function boundary",
                self.source.program.source,
            ));
        }
        let selection_functions: Vec<usize> = self
            .stages
            .iter()
            .filter_map(|stage| {
                stage
                    .source_selection
                    .as_ref()
                    .map(|selection| selection.function_index)
            })
            .collect();
        let mut functions = inner.functions.clone();
        functions.extend(self.stages.iter().map(|stage| stage.function_index));
        functions.extend(selection_functions.iter().copied());
        Ok(QcFunctionBoundary {
            functions,
            run: Box::new(move |call, execute| self.run_function(call, execute, &inner, &selection_functions)),
        })
    }

    /// Function dispatch.
    fn run_function(
        &self,
        call: &QcCallSite,
        execute: &dyn QcFunctionExecution,
        inner: &QcFunctionBoundary,
        selection_functions: &[usize],
    ) -> Result<(), QcError> {
        if selection_functions.contains(&call.function_index) {
            let gate = self.active.borrow().last().and_then(|frame| {
                frame.as_ref().and_then(|frame| {
                    frame.stage.source_selection.clone().map(|source| {
                        (
                            source,
                            frame.stage.function_index,
                            frame.recipient.actor.clone(),
                            frame.view(),
                        )
                    })
                })
            });
            if let Some((source, stage_index, recipient, view)) = gate {
                if source.function_index == call.function_index
                    && call.caller == stage_index
                    && source.calls.contains(&call.statement)
                    && !view.is_original
                    && !(self.primary_weapon_selected)(&recipient)
                {
                    self.validate()?;
                    let vm = self.vm()?;
                    let selected = vm.arg_float(1)?;
                    let mut item = None;
                    for weapon in &source.weapons {
                        if vm.global_float(weapon.word as usize)? == selected {
                            item = Some(weapon.item.clone());
                            break;
                        }
                    }
                    if !view.accepted
                        || vm.global_int(vm.global_offset("self")?)? != vm.entity_reference(view.recipient.slot)?
                    {
                        return Err(QcError::program(
                            "Original pickup selection lost its accepted recipient",
                            self.source.program.source,
                        ));
                    }
                    if let Some(item) = item {
                        let policy = self.policy.as_ref().and_then(|policy| policy());
                        let Some(policy) = policy else {
                            return Err(QcError::program(
                                "Original pickup has no selected weapon continuation",
                                self.source.program.source,
                            ));
                        };
                        policy.select(&recipient, &item)?;
                        self.validate()?;
                    }
                    execute.skip([0, 0, 0]);
                    return Ok(());
                }
            }
            if inner.functions.contains(&call.function_index) {
                return (inner.run)(call, execute);
            }
            return execute.run(None);
        }
        let Some(stage) = self
            .stages
            .iter()
            .find(|stage| stage.function_index == call.function_index)
            .cloned()
        else {
            return (inner.run)(call, execute);
        };
        self.run_pickup_call(execute, &stage)
    }

    /// Touch-function dispatch.
    fn run_pickup_call(&self, execute: &dyn QcFunctionExecution, stage: &QcPickupStage) -> Result<(), QcError> {
        let vm = self.vm()?;
        let pickup = self.actor(vm.global_int(vm.global_offset("self")?)?)?;
        let recipient = self.actor(vm.global_int(vm.global_offset("other")?)?)?;
        let (Some(pickup), Some(recipient)) = (pickup, recipient) else {
            execute.skip([0, 0, 0]);
            return Ok(());
        };
        let value = match &stage.descriptor {
            QcPickupStageDescriptor::Constant { .. } | QcPickupStageDescriptor::Cargo { .. } => None,
            QcPickupStageDescriptor::Str { field, .. } => Some(QcPickupValue::Str(
                vm.strings_get(vm.entity_int(pickup.slot, vm.field_offset(field)?)?)?,
            )),
            QcPickupStageDescriptor::Float { field, .. } => Some(QcPickupValue::Num(
                vm.entity_float(pickup.slot, vm.field_offset(field)?)?,
            )),
        };
        let descriptor = match &stage.descriptor {
            QcPickupStageDescriptor::Constant { value } | QcPickupStageDescriptor::Cargo { value, .. } => {
                Some(value.clone())
            }
            QcPickupStageDescriptor::Str { values, .. } | QcPickupStageDescriptor::Float { values, .. } => {
                let value = value
                    .ok_or_else(|| QcError::program("missing pickup discriminator", self.source.program.source))?;
                values.iter().find(|descriptor| descriptor.value == value).cloned()
            }
        };
        let Some(descriptor) = descriptor else {
            if self
                .active
                .borrow()
                .iter()
                .flatten()
                .any(|frame| frame.pickup.actor == pickup.actor)
            {
                execute.skip([0, 0, 0]);
                return Ok(());
            }
            let guard = FrameGuard::push(&self.active, None);
            let outcome = execute.run(None);
            guard.defuse();
            return outcome;
        };
        let mut source_effect = false;
        if let Some(effect) = stage.source_effect {
            source_effect = vm.global_float(effect.word as usize)? == effect.value;
        }
        let mut cargo = Vec::new();
        let mut new_weapon = false;
        if matches!(stage.descriptor, QcPickupStageDescriptor::Cargo { .. }) && !source_effect {
            let QcPickupStageDescriptor::Cargo { counters, weapons, .. } = &stage.descriptor else {
                return Err(QcError::program("missing cargo descriptor", self.source.program.source));
            };
            for counter in counters {
                cargo.push(PickupCargoEntry {
                    kind: PickupCargoKind::Counter,
                    item: counter.item.clone(),
                    count: vm.entity_float(pickup.slot, vm.field_offset(&counter.field)?)?,
                });
            }
            let bits = vm.entity_float(pickup.slot, vm.field_offset("items")?)?;
            if bits != 0.0 {
                let mut weapon = None;
                for candidate in weapons {
                    if vm.global_float(candidate.word as usize)? == bits {
                        weapon = Some(candidate);
                        break;
                    }
                }
                let Some(weapon) = weapon else {
                    return Err(QcError::program(
                        "Original backpack carried weapon is not qualified",
                        self.source.program.source,
                    ));
                };
                cargo.push(PickupCargoEntry {
                    kind: PickupCargoKind::Weapon,
                    item: weapon.item.clone(),
                    count: 1.0,
                });
                new_weapon = match &self.owns_weapon {
                    Some(owns) => !owns(&recipient.actor, &weapon.item),
                    None => {
                        float_to_wrapped_i32(vm.entity_float(recipient.slot, vm.field_offset("items")?)?)
                            & float_to_wrapped_i32(bits)
                            == 0
                    }
                };
            }
        }
        let count = match &descriptor.count {
            None => PickupCount::Default,
            Some(count) => PickupCount::Override {
                amount: self.scalar(count, pickup.slot)?,
            },
        };
        let mut dropped = matches!(stage.descriptor, QcPickupStageDescriptor::Cargo { .. });
        if let Some(scalar) = stage.dropped.as_ref() {
            dropped = dropped || self.scalar(scalar, pickup.slot)? != 0.0;
        }
        let offer = OriginalPickupOffer {
            recipient: recipient.actor.id().clone(),
            pickup: pickup.actor.id().clone(),
            source: self.source.slots.provider(),
            item: descriptor.item.clone(),
            default_resource: descriptor.resource.clone(),
            count,
            dropped,
            time: SourceTime::Seconds(vm.global_float(vm.global_offset("time")?)? as f32),
            cargo: if source_effect {
                Vec::new()
            } else if matches!(stage.descriptor, QcPickupStageDescriptor::Cargo { .. }) {
                cargo
            } else {
                Vec::new()
            },
            grant: if source_effect {
                Some(OriginalPickupGrant::SourceEffect)
            } else {
                None
            },
        };
        self.admission.run_source(&offer, &mut |selection, lifetime| {
            if matches!(selection, SourcePickupSelection::Stale) {
                execute.skip([0, 0, 0]);
                return Ok(());
            }
            // See the struct note: handles are popped by the guard
            // before this callback returns, on all paths.
            let selection = unsafe { extend_selection(selection) };
            let lifetime = unsafe { extend_lifetime(lifetime) };
            let guard = FrameGuard::push(
                &self.active,
                Some(PickupCall {
                    stage: stage.clone(),
                    descriptor: descriptor.clone(),
                    pickup: pickup.clone(),
                    recipient: recipient.clone(),
                    selection,
                    cancel_owner: execute.cancel_owner(),
                    lifetime,
                    new_weapon,
                    source_effect,
                    accepted: false,
                    consuming: false,
                    consumed: false,
                }),
            );
            let outcome = execute.run(None);
            guard.defuse();
            outcome
        })
    }

    /// Compose region boundaries (donor `composeRegions`).
    pub fn compose_regions(&'a self, inner: QcInlineBoundary<'a>) -> QcInlineBoundary<'a> {
        let regions: Vec<(usize, super::pickup_stage::QcPickupRegion)> = self
            .stages
            .iter()
            .flat_map(|stage| {
                stage
                    .regions
                    .iter()
                    .map(|region| (stage.function_index, region.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        let boundaries: Vec<QcInlineRegion> = regions.iter().map(|(_, region)| region.region).collect();
        let mut all = inner.regions.clone();
        all.extend(boundaries);
        QcInlineBoundary {
            regions: all,
            run: Box::new(move |region, execute| self.run_region(region, execute, &inner, &regions)),
        }
    }

    /// Region dispatch.
    #[allow(clippy::too_many_lines)]
    fn run_region(
        &self,
        region: &QcInlineRegion,
        execute: &dyn QcInlineContinuation,
        inner: &QcInlineBoundary,
        regions: &[(usize, super::pickup_stage::QcPickupRegion)],
    ) -> Result<(), QcError> {
        let Some((_, entry)) = regions
            .iter()
            .find(|(_, entry)| {
                entry.region.entry == region.entry && entry.region.function_index == region.function_index
            })
            .cloned()
        else {
            return (inner.run)(region, execute);
        };
        let frame = self
            .active
            .borrow()
            .last()
            .and_then(|frame| frame.as_ref().map(PickupCall::view));
        let Some(frame) = frame else {
            return execute.run();
        };
        if frame.stage.function_index != region.function_index {
            return execute.run();
        }
        if self
            .projection
            .borrow()
            .as_ref()
            .is_some_and(|projected| projected.region.entry == region.entry)
        {
            return execute.run();
        }
        self.validate()?;
        let vm = self.vm()?;
        let caller_current = || -> Result<bool, QcError> {
            let expected_self = if frame.consumed || entry.recipient_self {
                frame.recipient.slot
            } else {
                frame.pickup_slot
            };
            Ok(
                vm.global_int(vm.global_offset("self")?)? == vm.entity_reference(expected_self)?
                    && vm.global_int(vm.global_offset("other")?)? == vm.entity_reference(frame.recipient.slot)?,
            )
        };
        if !caller_current()? {
            return Err(QcError::cancelled(frame.cancel_owner, [0, 0, 0]));
        }
        if matches!(entry.operation, QcPickupOperation::SourceEffect) {
            if !frame.source_effect || !frame.is_original {
                return Err(QcError::cancelled(frame.cancel_owner, [0, 0, 0]));
            }
            execute.run()?;
            self.validate()?;
            if let Some(frame) = self.active.borrow_mut().last_mut().and_then(|frame| frame.as_mut()) {
                frame.accepted = true;
            }
            return Ok(());
        }
        if matches!(entry.operation, QcPickupOperation::Consume) {
            if frame.source_effect && !frame.accepted {
                return Err(QcError::program(
                    "Original source effect did not reach its grant",
                    self.source.program.source,
                ));
            }
            if let Some(frame) = self.active.borrow_mut().last_mut().and_then(|frame| frame.as_mut()) {
                frame.consuming = true;
            }
            let lifetime = frame.lifetime;
            // The donor passes the live continuation as `remove` and its
            // lifetime implementation runs it synchronously; the Rust
            // implementation must do the same (see `extend_execute`).
            let remove = unsafe { extend_execute(execute) };
            lifetime.consume_pickup(Box::new(move || {
                if let Err(error) = remove.run() {
                    // The contract `remove` cannot fail; unwind with the
                    // guest error exactly as the donor `throw` does.
                    std::panic::panic_any(error);
                }
            }));
            if let Some(frame) = self.active.borrow_mut().last_mut().and_then(|frame| frame.as_mut()) {
                frame.consumed = true;
                frame.consuming = false;
            }
            self.validate()?;
            return Ok(());
        }
        let policy = self.policy.as_ref().and_then(|policy| policy());
        if let QcPickupOperation::Counter { field, item } = &entry.operation {
            if !(self.primary_weapon_selected)(&frame.recipient.actor) && policy.is_some() {
                let policy =
                    policy.ok_or_else(|| QcError::program("missing pickup policy", self.source.program.source))?;
                let operation_field = field.clone();
                let operation_item = item.clone();
                let recipient = frame.recipient.clone();
                policy.counter(&recipient.actor, &operation_item, &mut |count| {
                    self.validate()?;
                    if !fround(count).is_finite() || self.projection.borrow().is_some() {
                        return Err(QcError::program(
                            "Invalid original pickup counter projection",
                            self.source.program.source,
                        ));
                    }
                    let vm = self.vm()?;
                    let word = vm.field_offset(&operation_field)?;
                    let saved = vm.entity_int(recipient.slot, word)?;
                    *self.projection.borrow_mut() = Some(PickupProjection {
                        region: *region,
                        reference: vm.entity_reference(recipient.slot)?,
                        word,
                        value: None,
                    });
                    let outcome = (|| -> Result<(), QcError> {
                        vm.set_entity_float(recipient.slot, word, count)?;
                        vm.execute_region(region, 0)?;
                        Ok(())
                    })();
                    let value = self.projection.borrow().as_ref().and_then(|projected| projected.value);
                    vm.set_entity_int(recipient.slot, word, saved)?;
                    *self.projection.borrow_mut() = None;
                    outcome?;
                    self.validate()?;
                    Ok(value)
                })?;
                self.validate()?;
                return execute.skip_to_join();
            }
        }
        if frame.is_original {
            return execute.run();
        }
        if matches!(entry.operation, QcPickupOperation::WeaponSelection)
            && (self.primary_weapon_selected)(&frame.recipient.actor)
        {
            return execute.run();
        }
        if matches!(
            entry.operation,
            QcPickupOperation::Decision { .. } | QcPickupOperation::Admission
        ) {
            if frame.accepted {
                return Err(QcError::program(
                    "Original pickup reached more than one recipient decision",
                    self.source.program.source,
                ));
            }
            // The replacement grant runs resource code that never
            // reenters this binding, so the short borrow is safe.
            let granted = self.active.borrow().last().and_then(|frame| {
                frame.as_ref().and_then(|frame| match &frame.selection {
                    SourcePickupSelection::Replacement { grant, .. } => Some(grant()),
                    _ => None,
                })
            });
            if !frame.is_replacement || granted != Some(OriginalPickupOutcome::Accepted) {
                return Err(QcError::cancelled(frame.cancel_owner, [0, 0, 0]));
            }
            self.validate()?;
            if !caller_current()? {
                return Err(QcError::cancelled(frame.cancel_owner, [0, 0, 0]));
            }
            if let Some(frame) = self.active.borrow_mut().last_mut().and_then(|frame| frame.as_mut()) {
                frame.accepted = true;
            }
            if matches!(entry.operation, QcPickupOperation::Admission) {
                return execute.run();
            }
            if let QcPickupOperation::Decision { word, accepted } = entry.operation {
                vm.set_global_float(word as usize, accepted)?;
            }
        } else if !frame.accepted {
            return Err(QcError::program(
                "Original pickup grant has no accepted recipient decision",
                self.source.program.source,
            ));
        }
        if let QcPickupOperation::CargoOwnership { word } = entry.operation {
            vm.set_global_float(word as usize, if frame.new_weapon { 1.0 } else { 0.0 })?;
        }
        if let QcPickupOperation::CargoCurrent { word } = entry.operation {
            let current = policy.and_then(|policy| policy.current(&frame.recipient.actor));
            let source = frame.stage.source_selection.as_ref().and_then(|selection| {
                selection
                    .weapons
                    .iter()
                    .find(|weapon| Some(&weapon.item) == current.as_ref())
            });
            vm.set_global_float(
                word as usize,
                match source {
                    Some(source) => vm.global_float(source.word as usize)?,
                    None => 0.0,
                },
            )?;
        }
        execute.skip_to_join()
    }
}

/// Owned frame snapshot for region dispatch (the selection closures
/// stay borrowed in the frame and are invoked through short reborrows).
struct FrameView {
    stage: QcPickupStage,
    recipient: SourceActor,
    pickup_slot: usize,
    is_original: bool,
    is_replacement: bool,
    accepted: bool,
    consumed: bool,
    new_weapon: bool,
    source_effect: bool,
    cancel_owner: u64,
    lifetime: &'static dyn SourcePickupLifetime,
}

impl PickupCall {
    /// Snapshot the owned frame state.
    fn view(&self) -> FrameView {
        FrameView {
            stage: self.stage.clone(),
            recipient: self.recipient.clone(),
            pickup_slot: self.pickup.slot,
            is_original: matches!(self.selection, SourcePickupSelection::Original),
            is_replacement: matches!(self.selection, SourcePickupSelection::Replacement { .. }),
            accepted: self.accepted,
            consumed: self.consumed,
            new_weapon: self.new_weapon,
            source_effect: self.source_effect,
            cancel_owner: self.cancel_owner,
            lifetime: self.lifetime,
        }
    }
}
