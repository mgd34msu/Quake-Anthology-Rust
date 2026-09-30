//! QVM weapon stage: original dispatcher decisions shared with source callers.
//!
//! Provenance: `src/compat/qvm/mod-weapon-stage.ts`.
//!
//! Absorbs the pure-Rust types of `src/contracts/qvm-mod-items.ts`.
//! Held-weapon declarations live with [`super::primary_presentation_profile`];
//! source calls and input pointers reuse [`super::mod_provider`]. Invocation
//! hooks (`bindInvocation`, branch bindings, region evaluation, cancellation
//! scopes) are interpreter-owned: this port exposes the same decisions as an
//! explicit state machine driven by [`WeaponStageHost`] (scope enter/exit,
//! predicate decisions, request gating, continuation projection), which the
//! interpreter integration calls at the donor's hook points.

use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use super::game_data::{QvmCancellationScope, QvmFunctionCall, QvmModule};
use super::mod_provider::{
    InputPointerKind, ModReturns, QvmImage, QvmModInputPointer, QvmModSourceCall, QvmOpcode, QvmRegionEvaluation,
    QVM_MAX_PRIVATE_ARGUMENT_WORDS,
};
use crate::error::GuestError;

// ---------------------------------------------------------------------------
// Item contract types (`src/contracts/qvm-mod-items.ts`).
// ---------------------------------------------------------------------------

/// One source word field.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmItemField {
    /// Record id.
    pub record: String,
    /// Field offset.
    pub offset: usize,
}

/// Test comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TestComparison {
    /// Equality.
    Equals,
    /// At most.
    AtMost,
}

/// One source state predicate.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmItemTest {
    /// Field.
    pub field: QvmItemField,
    /// Mask, if any.
    pub mask: Option<u32>,
    /// Comparison.
    pub comparison: TestComparison,
    /// Value.
    pub value: i32,
}

/// Override comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OverrideComparison {
    /// Equality.
    Equals,
    /// Inequality.
    NotEquals,
}

/// Capacity selector override.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CapacityOverride {
    /// Selector address.
    pub address: usize,
    /// Comparison.
    pub comparison: OverrideComparison,
    /// Value (int32-checked at validation, mirroring the donor).
    pub value: i64,
    /// Constant instruction.
    pub instruction: usize,
}

/// Item capacity source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QvmItemCapacity {
    /// Constant capacity.
    Constant(i32),
    /// Live field capacity.
    Field(QvmItemField),
    /// Source selector capacity.
    Source {
        /// Constant instruction.
        instruction: usize,
        /// Overrides.
        overrides: Vec<CapacityOverride>,
    },
}

/// Packed item bit.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackedItem {
    /// Item identity.
    pub item: String,
    /// Bit mask.
    pub mask: u32,
}

/// Item storage declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QvmItemStorage {
    /// Counter storage.
    Counter {
        /// Field.
        field: QvmItemField,
        /// Item identity.
        item: String,
        /// Capacity.
        capacity: QvmItemCapacity,
    },
    /// Packed-bits storage.
    Bits {
        /// Field.
        field: QvmItemField,
        /// Private mask.
        private_mask: u32,
        /// Packed items.
        items: Vec<PackedItem>,
    },
}

impl QvmItemStorage {
    /// Storage field.
    #[must_use]
    pub fn field(&self) -> &QvmItemField {
        match self {
            Self::Counter { field, .. } | Self::Bits { field, .. } => field,
        }
    }

    /// Item identities packed into this storage entry.
    #[must_use]
    pub fn stored_items(&self) -> Vec<&str> {
        match self {
            Self::Counter { item, .. } => vec![item.as_str()],
            Self::Bits { items, .. } => items.iter().map(|entry| entry.item.as_str()).collect(),
        }
    }

    /// Build counter storage.
    #[must_use]
    pub fn counter(field: QvmItemField, item: String, capacity: QvmItemCapacity) -> Self {
        Self::Counter { field, item, capacity }
    }

    /// Build packed-bits storage.
    #[must_use]
    pub fn bits(field: QvmItemField, private_mask: u32, items: Vec<PackedItem>) -> Self {
        Self::Bits {
            field,
            private_mask,
            items,
        }
    }
}

/// Source actor selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmWeaponActor {
    /// Record id.
    pub record: String,
    /// Pointer.
    pub pointer: QvmModInputPointer,
}

/// Dispatcher head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatcherHead {
    /// Entry instruction.
    pub entry: usize,
    /// Actor selector.
    pub actor: QvmWeaponActor,
}

/// Stage predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StagePredicate {
    /// Instruction.
    pub instruction: usize,
    /// Decision while unselected.
    pub unselected: bool,
}

/// Selection value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SelectionValue {
    /// Source value.
    pub value: i32,
    /// Item identity.
    pub item: String,
}

/// Weapon selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageSelection {
    /// Selection field.
    pub field: QvmItemField,
    /// Declared values.
    pub values: Vec<SelectionValue>,
}

/// Weapon request gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageRequest {
    /// Entry instruction.
    pub entry: usize,
    /// Requested-weapon argument.
    pub argument: usize,
    /// Accepted-state tests.
    pub accepted: Vec<QvmItemTest>,
}

/// Invocation-owned weapon decisions (the dispatcher head of a stage).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmWeaponDispatcherDefinition {
    /// Dispatcher head.
    pub dispatcher: DispatcherHead,
    /// Predicates.
    pub predicates: Vec<StagePredicate>,
    /// Settled-state tests.
    pub settled: Vec<QvmItemTest>,
    /// Selection.
    pub selection: StageSelection,
    /// Request gate.
    pub request: StageRequest,
}

/// Continuation movement projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageProjection {
    /// Movement pointer.
    pub movement: QvmModInputPointer,
    /// Caller layout length.
    pub byte_length: usize,
    /// Minimums offset.
    pub minimum: usize,
    /// Maximums offset.
    pub maximum: usize,
    /// View-height field.
    pub view_height: QvmItemField,
    /// Ground field.
    pub ground: QvmItemField,
}

/// Continuation call site.
#[derive(Debug, Clone, PartialEq)]
pub struct ContinuationCall {
    /// Call instruction.
    pub instruction: usize,
    /// Source call.
    pub call: QvmModSourceCall,
}

/// Weapon continuation.
#[derive(Debug, Clone, PartialEq)]
pub struct StageContinuation {
    /// Entry instruction.
    pub entry: usize,
    /// Actor selector.
    pub actor: QvmWeaponActor,
    /// Boundary instruction.
    pub instruction: usize,
    /// Original taken direction.
    pub original_taken: bool,
    /// Continuation conditions.
    pub when: Vec<QvmItemTest>,
    /// Predicates.
    pub predicates: Vec<StagePredicate>,
    /// Movement projection.
    pub projection: StageProjection,
    /// Ordered calls.
    pub calls: Vec<ContinuationCall>,
}

/// Full weapon stage.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmWeaponStage {
    /// Dispatcher head.
    pub dispatcher: DispatcherHead,
    /// Predicates.
    pub predicates: Vec<StagePredicate>,
    /// Settled-state tests.
    pub settled: Vec<QvmItemTest>,
    /// Selection.
    pub selection: StageSelection,
    /// Request gate.
    pub request: StageRequest,
    /// Continuation.
    pub continuation: StageContinuation,
}

impl QvmWeaponStage {
    /// Dispatcher head of this stage.
    #[must_use]
    pub fn dispatcher_definition(&self) -> QvmWeaponDispatcherDefinition {
        QvmWeaponDispatcherDefinition {
            dispatcher: self.dispatcher.clone(),
            predicates: self.predicates.clone(),
            settled: self.settled.clone(),
            selection: self.selection.clone(),
            request: self.request.clone(),
        }
    }
}

/// Item admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemAdmission {
    /// Add.
    Add,
    /// Replace primary.
    ReplacePrimary,
}

/// Item action calls.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ItemActions {
    /// Use call, if any.
    pub use_: Option<QvmModSourceCall>,
    /// Drop call, if any.
    pub drop_: Option<QvmModSourceCall>,
}

/// Item definition kind.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmItemDefinitionKind {
    /// Counter item.
    Counter,
    /// Weapon item.
    Weapon {
        /// Held-weapon declaration, if any.
        held: Option<super::primary_presentation_profile::HeldWeaponDeclaration>,
        /// Ammo item, if any.
        ammo: Option<String>,
    },
}

/// Item definition.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmItemDefinition {
    /// Item identity.
    pub item: String,
    /// Label.
    pub label: String,
    /// Icon path, if any (`None` covers absent and null).
    pub icon: Option<String>,
    /// Admission.
    pub admission: ItemAdmission,
    /// Actions, if any.
    pub actions: Option<ItemActions>,
    /// Kind.
    pub kind: QvmItemDefinitionKind,
}

impl QvmItemDefinition {
    /// Declared action calls (use, then drop).
    #[must_use]
    pub fn action_calls(&self) -> Vec<&QvmModSourceCall> {
        self.actions
            .iter()
            .flat_map(|actions| actions.use_.iter().chain(actions.drop_.iter()))
            .collect()
    }
}

/// Weapon input clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponInput {
    /// Input entry.
    pub entry: usize,
    /// Clock field.
    pub clock: QvmItemField,
}

/// Declared weapon consumer.
#[derive(Debug, Clone, PartialEq)]
pub struct ModItemWeapons {
    /// Input.
    pub input: WeaponInput,
    /// Stage.
    pub stage: QvmWeaponStage,
}

/// Item definitions (mirror of `QvmModItems`).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModItems {
    /// Definitions.
    pub definitions: Vec<QvmItemDefinition>,
    /// Storage.
    pub storage: Vec<QvmItemStorage>,
    /// Weapon consumer, if any.
    pub weapons: Option<ModItemWeapons>,
}

// ---------------------------------------------------------------------------
// Stage validation.
// ---------------------------------------------------------------------------

fn function_end(image: &QvmImage, entry: usize) -> Result<usize, GuestError> {
    if image
        .instruction(entry)
        .is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter)
    {
        return Err(GuestError::invalid("QVM weapon entry is not an original function"));
    }
    Ok(image.function_end(entry))
}

/// Validate a weapon dispatcher head.
pub fn validate_qvm_weapon_dispatcher(
    stage: &QvmWeaponDispatcherDefinition,
    image: &QvmImage,
) -> Result<(), GuestError> {
    let end = function_end(image, stage.dispatcher.entry)?;
    let mut seen = std::collections::HashSet::new();
    for predicate in &stage.predicates {
        let instruction = image.instruction(predicate.instruction);
        if predicate.instruction <= stage.dispatcher.entry
            || predicate.instruction >= end
            || !seen.insert(predicate.instruction)
            || instruction.is_none_or(|value| !value.opcode.is_branch())
        {
            return Err(GuestError::invalid(
                "QVM weapon predicate is not a distinct conditional in its original dispatcher",
            ));
        }
    }
    function_end(image, stage.request.entry)?;
    if stage.predicates.is_empty()
        || stage.settled.is_empty()
        || stage.request.accepted.is_empty()
        || stage.request.argument >= QVM_MAX_PRIVATE_ARGUMENT_WORDS
    {
        return Err(GuestError::invalid(
            "QVM weapon stage lacks its original decisions or settlement state",
        ));
    }
    Ok(())
}

/// Validate a full weapon stage.
pub fn validate_qvm_weapon_stage(stage: &QvmWeaponStage, image: &QvmImage) -> Result<(), GuestError> {
    validate_qvm_weapon_dispatcher(&stage.dispatcher_definition(), image)?;
    let continuation = &stage.continuation;
    let limit = function_end(image, continuation.entry)?;
    let branch = image.instruction(continuation.instruction);
    if continuation.when.is_empty() {
        return Err(GuestError::invalid(
            "QVM weapon continuation lacks its source mode conditions",
        ));
    }
    let mut decisions = std::collections::HashSet::from([continuation.instruction]);
    for predicate in &continuation.predicates {
        let instruction = image.instruction(predicate.instruction);
        if predicate.instruction <= continuation.entry
            || predicate.instruction >= limit
            || !decisions.insert(predicate.instruction)
            || instruction.is_none_or(|value| !value.opcode.is_branch())
        {
            return Err(GuestError::invalid(
                "QVM weapon continuation predicate is not a distinct original conditional",
            ));
        }
    }
    if continuation.instruction <= continuation.entry
        || continuation.instruction >= limit
        || branch.is_none_or(|value| !value.opcode.is_branch() || value.operand_width != 4)
    {
        return Err(GuestError::invalid(
            "QVM weapon continuation lacks an original conditional boundary",
        ));
    }
    let mut pc = if continuation.original_taken {
        continuation.instruction + 1
    } else {
        branch.expect("checked branch").operand.max(0) as usize
    };
    let mut visited = std::collections::HashSet::new();
    loop {
        if pc <= continuation.entry || pc >= limit || !visited.insert(pc) {
            return Err(GuestError::invalid(
                "QVM weapon continuation does not take an original return edge",
            ));
        }
        let instruction = image.instruction(pc);
        if instruction.is_some_and(|value| value.opcode == QvmOpcode::OpLeave) {
            break;
        }
        if instruction.is_some_and(|value| value.opcode == QvmOpcode::OpPush) {
            pc += 1;
            continue;
        }
        if instruction.is_some_and(|value| value.opcode == QvmOpcode::OpConst)
            && image
                .instruction(pc + 1)
                .is_some_and(|value| value.opcode == QvmOpcode::OpJump)
        {
            pc = instruction.expect("checked const").operand.max(0) as usize;
            continue;
        }
        return Err(GuestError::invalid(
            "QVM weapon continuation return edge has source side effects",
        ));
    }
    let mut previous = continuation.instruction;
    for value in &continuation.calls {
        let target = value.instruction.checked_sub(1).and_then(|at| image.instruction(at));
        if value.instruction <= previous
            || value.instruction >= limit
            || image
                .instruction(value.instruction)
                .is_none_or(|instruction| instruction.opcode != QvmOpcode::OpCall)
            || target
                .is_none_or(|target| target.opcode != QvmOpcode::OpConst || target.operand != value.call.entry as i32)
            || !value.call.arguments.is_empty()
            || !value.call.globals.is_empty()
            || value.call.returns != ModReturns::Void
        {
            return Err(GuestError::invalid(
                "QVM weapon continuation differs from its ordered original no-argument calls",
            ));
        }
        function_end(image, value.call.entry)?;
        previous = value.instruction;
    }
    if !continuation
        .calls
        .iter()
        .any(|value| value.call.entry == stage.dispatcher.entry)
    {
        return Err(GuestError::invalid(
            "QVM continuation omits its original weapon dispatcher",
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Stage runtime.
// ---------------------------------------------------------------------------

/// Branch decision of one predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchDecision {
    /// Proceed with a direction.
    Take(bool),
    /// Cancel the calling function.
    Cancel,
}

/// Authoritative posture for continuation projection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponPosture {
    /// Bounds minimums.
    pub bounds_min: Vec3,
    /// Bounds maximums.
    pub bounds_max: Vec3,
    /// View height.
    pub view_height: i32,
    /// Ground entity number.
    pub ground: i32,
}

/// Host services of the weapon stage.
pub trait WeaponStageHost {
    /// Resolve a record pointer for an actor.
    fn pointer(&self, actor: &ActorId, record: &str) -> Result<usize, GuestError>;
    /// Whether an actor is live.
    fn live(&self, actor: &ActorId) -> bool;
    /// Whether an actor has this source selected.
    fn selected(&self, actor: &ActorId) -> bool;
    /// Read one source word.
    fn read_i32(&self, address: usize) -> Result<i32, GuestError>;
    /// Write one source word.
    fn write_i32(&mut self, address: usize, value: i32) -> Result<(), GuestError>;
    /// Write one source float.
    fn write_f32(&mut self, address: usize, value: f32) -> Result<(), GuestError>;
    /// Note an attempted weapon request.
    fn attempted(&mut self, actor: &ActorId, value: i32);
    /// Note an accepted weapon request.
    fn accepted(&mut self, actor: &ActorId, value: i32);
    /// Note a completed dispatch.
    fn completed(&mut self, actor: &ActorId, reached_attack_decision: bool);
    /// Evaluate a region in the dispatcher.
    fn evaluate_region(&mut self, region: &QvmRegionEvaluation, inputs: &[i32]) -> Result<i32, GuestError>;
    /// Call the dispatcher entry with no arguments.
    fn call_dispatcher(&mut self, entry: usize) -> Result<i32, GuestError>;
    /// Resolve an input pointer against call words.
    fn resolve_input_pointer(&self, pointer: &QvmModInputPointer, words: &[i32]) -> Result<usize, GuestError>;
    /// Authoritative posture of an actor.
    fn posture(&self, actor: &ActorId) -> Result<WeaponPosture, GuestError>;
    /// Invoke a continuation call for an actor.
    fn invoke_source(&mut self, actor: &ActorId, call: &QvmModSourceCall) -> Result<i32, GuestError>;
    /// Mint a fresh cancellation scope token.
    fn fresh_scope(&mut self) -> u64;
    /// Cancel a scope token.
    fn cancel_scope(&mut self, scope: u64);
}

/// Donor dispatcher operations (mirror of `QvmWeaponDispatcherOperations`).
///
/// Concrete over the interpreter-owning workers' types so game callers
/// construct it literally. Actor resolution failures surface as
/// [`GuestError`] where the donor returned null.
pub struct QvmWeaponDispatcherOperations {
    /// Source module.
    pub module: QvmModule,
    /// Resolve the source actor of a call.
    pub actor:
        Rc<dyn Fn(&super::item_storage::QvmWeaponActor, &mut QvmFunctionCall) -> Result<Option<ActorId>, GuestError>>,
    /// Resolve a record pointer for an actor.
    pub pointer: Rc<dyn Fn(&ActorId, &str) -> Result<usize, GuestError>>,
    /// Whether an actor is live.
    pub live: Rc<dyn Fn(&ActorId) -> bool>,
    /// Whether an actor has this source selected.
    pub selected: Rc<dyn Fn(&ActorId) -> bool>,
    /// Cancellation scope of a call.
    pub cancellation: Rc<dyn Fn(&ActorId, &mut QvmFunctionCall) -> QvmCancellationScope>,
    /// Note an attempted weapon request.
    pub attempted: Rc<dyn Fn(&ActorId, i32)>,
    /// Note an accepted weapon request.
    pub accepted: Rc<dyn Fn(&ActorId, i32)>,
    /// Note a completed dispatch.
    pub completed: Rc<dyn Fn(&ActorId, bool)>,
    /// Prepare hook, if any.
    pub prepare: Option<Rc<dyn Fn(&ActorId, &mut QvmFunctionCall) -> Option<Box<dyn FnOnce()>>>>,
}

/// Process-unique dispatcher scope tokens.
static NEXT_DISPATCH_SCOPE: AtomicU64 = AtomicU64::new(1);

impl WeaponStageHost for QvmWeaponDispatcherOperations {
    fn pointer(&self, actor: &ActorId, record: &str) -> Result<usize, GuestError> {
        (self.pointer)(actor, record)
    }

    fn live(&self, actor: &ActorId) -> bool {
        (self.live)(actor)
    }

    fn selected(&self, actor: &ActorId) -> bool {
        (self.selected)(actor)
    }

    fn read_i32(&self, address: usize) -> Result<i32, GuestError> {
        self.module.memory().read_i32(address)
    }

    fn write_i32(&mut self, address: usize, value: i32) -> Result<(), GuestError> {
        self.module.memory().write_i32(address, value)
    }

    fn write_f32(&mut self, address: usize, value: f32) -> Result<(), GuestError> {
        self.module.memory().write_f32(address, value)
    }

    fn attempted(&mut self, actor: &ActorId, value: i32) {
        (self.attempted)(actor, value);
    }

    fn accepted(&mut self, actor: &ActorId, value: i32) {
        (self.accepted)(actor, value);
    }

    fn completed(&mut self, actor: &ActorId, reached_attack_decision: bool) {
        (self.completed)(actor, reached_attack_decision);
    }

    fn evaluate_region(&mut self, region: &QvmRegionEvaluation, inputs: &[i32]) -> Result<i32, GuestError> {
        let region = super::game_data::QvmRegionEvaluation {
            entry: region.entry,
            join: region.join,
            inputs: region.inputs.clone(),
            result: region.result,
        };
        Ok(self.module.evaluate_region(&[], 0, &region, inputs))
    }

    fn call_dispatcher(&mut self, entry: usize) -> Result<i32, GuestError> {
        Ok(self.module.invoke_source_callback(entry, &[]))
    }

    fn resolve_input_pointer(&self, pointer: &QvmModInputPointer, words: &[i32]) -> Result<usize, GuestError> {
        let memory = self.module.memory();
        let mut found = match pointer.kind {
            InputPointerKind::Argument { index } => words
                .get(index)
                .copied()
                .ok_or_else(|| GuestError::invalid("QVM weapon input pointer names a missing call word"))?,
            InputPointerKind::Global { address } => memory.read_i32(address)?,
        };
        for offset in &pointer.indirections {
            let base = usize::try_from(found)
                .map_err(|_| GuestError::invalid("QVM weapon input pointer left guest memory"))?;
            found = memory.read_i32(
                base.checked_add(*offset)
                    .ok_or_else(|| GuestError::invalid("QVM weapon input pointer left guest memory"))?,
            )?;
        }
        let base =
            usize::try_from(found).map_err(|_| GuestError::invalid("QVM weapon input pointer left guest memory"))?;
        base.checked_add(pointer.offset)
            .ok_or_else(|| GuestError::invalid("QVM weapon input pointer left guest memory"))
    }

    fn posture(&self, _actor: &ActorId) -> Result<WeaponPosture, GuestError> {
        Err(GuestError::invalid(
            "QVM weapon posture requires the staging integration's movement record",
        ))
    }

    fn invoke_source(&mut self, actor: &ActorId, call: &QvmModSourceCall) -> Result<i32, GuestError> {
        let _ = actor;
        if !call.arguments.is_empty() {
            return Err(GuestError::invalid("QVM weapon continuation calls take no arguments"));
        }
        Ok(self.module.invoke_source_callback(call.entry, &[]))
    }

    fn fresh_scope(&mut self) -> u64 {
        NEXT_DISPATCH_SCOPE.fetch_add(1, Ordering::Relaxed)
    }

    fn cancel_scope(&mut self, _scope: u64) {}
}

/// Invocation-owned weapon decisions.
pub struct QvmWeaponDispatcher<H: WeaponStageHost = QvmWeaponDispatcherOperations> {
    host: H,
    definition: QvmWeaponDispatcherDefinition,
    dispatchers: Vec<ActorId>,
    evaluations: Vec<WeaponEvaluation>,
    reached_attack_decision: bool,
}

struct WeaponEvaluation {
    actor: ActorId,
    region: QvmRegionEvaluation,
    inputs: Vec<i32>,
    entered: bool,
}

impl<H: WeaponStageHost> QvmWeaponDispatcher<H> {
    /// Create a dispatcher.
    pub fn new(definition: QvmWeaponDispatcherDefinition, host: H) -> Self {
        Self {
            host,
            definition,
            dispatchers: Vec::new(),
            evaluations: Vec::new(),
            reached_attack_decision: false,
        }
    }

    /// Borrow the host.
    pub fn host(&self) -> &H {
        &self.host
    }

    /// Mutably borrow the host.
    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }

    fn scalar(&self, actor: &ActorId, field: &QvmItemField) -> Result<i32, GuestError> {
        Ok(self
            .host
            .read_i32(self.host.pointer(actor, &field.record)? + field.offset)?)
    }

    fn test(&self, actor: &ActorId, test: &QvmItemTest) -> Result<bool, GuestError> {
        let scalar = self.scalar(actor, &test.field)?;
        let value = test.mask.map_or(scalar, |mask| scalar & (mask as i32));
        Ok(if test.comparison == TestComparison::Equals {
            value == test.value
        } else {
            value <= test.value
        })
    }

    /// Whether an actor's weapon state is settled.
    pub fn settled(&self, actor: &ActorId) -> Result<bool, GuestError> {
        if !self.host.live(actor) {
            return Ok(false);
        }
        for test in &self.definition.settled {
            if !self.test(actor, test)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Active weapon item of an actor, if any.
    pub fn active(&self, actor: &ActorId) -> Result<Option<String>, GuestError> {
        let selected = self.scalar(actor, &self.definition.selection.field)?;
        if selected == 0 {
            return Ok(None);
        }
        let value = self
            .definition
            .selection
            .values
            .iter()
            .find(|value| value.value == selected);
        match value {
            Some(value) => Ok(Some(value.item.clone())),
            None => Err(GuestError::invalid("Original QVM selected an undeclared source weapon")),
        }
    }

    /// Whether an actor matches every test.
    pub fn matches(&self, actor: &ActorId, tests: &[QvmItemTest]) -> Result<bool, GuestError> {
        for test in tests {
            if !self.test(actor, test)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Enter a dispatcher invocation for an actor.
    pub fn begin_dispatch(&mut self, actor: ActorId) {
        self.reached_attack_decision = false;
        self.dispatchers.push(actor);
    }

    /// Decide one dispatcher predicate.
    pub fn decide_predicate(&mut self, actor: &ActorId, predicate: &StagePredicate, original: bool) -> BranchDecision {
        if !self.host.live(actor) {
            return BranchDecision::Cancel;
        }
        self.reached_attack_decision = true;
        BranchDecision::Take(if self.host.selected(actor) {
            original
        } else {
            predicate.unselected
        })
    }

    /// Finish a dispatcher invocation.
    pub fn end_dispatch(&mut self, actor: &ActorId) {
        if let Some(index) = self.dispatchers.iter().rposition(|scope| scope == actor) {
            self.dispatchers.remove(index);
        }
        if self.host.live(actor) {
            self.host.completed(actor, self.reached_attack_decision);
        }
    }

    /// Innermost live dispatcher scope, if any.
    pub fn current_dispatch(&self) -> Option<ActorId> {
        self.dispatchers.last().filter(|actor| self.host.live(actor)).cloned()
    }

    /// Gate a weapon request, noting attempts.
    pub fn request_gate(&mut self, actor: &ActorId, requested: i32) -> Result<bool, GuestError> {
        let accepted = self.matches(actor, &self.definition.request.accepted.clone())?;
        if !accepted {
            self.host.attempted(actor, requested);
        }
        Ok(accepted)
    }

    /// Finish a weapon request, noting late acceptance.
    pub fn request_finish(&mut self, actor: &ActorId, requested: i32, was_accepted: bool) -> Result<(), GuestError> {
        if !was_accepted && self.host.live(actor) && self.matches(actor, &self.definition.request.accepted.clone())? {
            self.host.accepted(actor, requested);
        }
        Ok(())
    }

    /// Evaluate a region inside the dispatcher.
    pub fn evaluate(
        &mut self,
        actor: &ActorId,
        region: &QvmRegionEvaluation,
        inputs: &[i32],
    ) -> Result<i32, GuestError> {
        if !self.host.live(actor) {
            return Err(GuestError::invalid(
                "QVM weapon evaluation requires its current source actor",
            ));
        }
        let entry = self.definition.dispatcher.entry;
        self.evaluations.push(WeaponEvaluation {
            actor: actor.clone(),
            region: region.clone(),
            inputs: inputs.to_vec(),
            entered: false,
        });
        let outcome = self.host.call_dispatcher(entry);
        self.evaluations.pop();
        outcome
    }

    /// Enter a dispatcher invocation that may serve a queued evaluation.
    pub fn enter_evaluation(&mut self, actor: &ActorId) -> Result<Option<i32>, GuestError> {
        let Some(evaluation) = self.evaluations.last_mut() else {
            return Ok(None);
        };
        if evaluation.entered {
            return Ok(None);
        }
        evaluation.entered = true;
        if evaluation.actor != *actor || !self.host.live(actor) {
            return Err(GuestError::invalid(
                "QVM weapon evaluation lost its original source actor",
            ));
        }
        let region = evaluation.region.clone();
        let inputs = evaluation.inputs.clone();
        Ok(Some(self.host.evaluate_region(&region, &inputs)?))
    }
}

/// Original source setup and weapon code sharing the live caller's input.
pub struct QvmModWeaponStage<H: WeaponStageHost> {
    dispatcher: QvmWeaponDispatcher<H>,
    definition: QvmWeaponStage,
    applications: Vec<ActorId>,
    inputs: Vec<StageInput>,
}

struct StageInput {
    actor: ActorId,
    scope: Option<u64>,
    entered: bool,
}

impl<H: WeaponStageHost> QvmModWeaponStage<H> {
    /// Create a weapon stage.
    pub fn new(definition: QvmWeaponStage, host: H) -> Self {
        let dispatcher = QvmWeaponDispatcher::new(definition.dispatcher_definition(), host);
        Self {
            dispatcher,
            definition,
            applications: Vec::new(),
            inputs: Vec::new(),
        }
    }

    /// Borrow the dispatcher.
    pub fn dispatcher(&self) -> &QvmWeaponDispatcher<H> {
        &self.dispatcher
    }

    /// Mutably borrow the dispatcher.
    pub fn dispatcher_mut(&mut self) -> &mut QvmWeaponDispatcher<H> {
        &mut self.dispatcher
    }

    /// Whether an actor's weapon state is settled.
    pub fn settled(&self, actor: &ActorId) -> Result<bool, GuestError> {
        self.dispatcher.settled(actor)
    }

    /// Active weapon item of an actor, if any.
    pub fn active(&self, actor: &ActorId) -> Result<Option<String>, GuestError> {
        self.dispatcher.active(actor)
    }

    /// Resolve the live application actor selected by a pointer.
    pub fn actor_for(&self, source: &QvmWeaponActor, words: &[i32]) -> Result<Option<ActorId>, GuestError> {
        let Some(actor) = self.applications.last().cloned() else {
            return Ok(None);
        };
        if !self.dispatcher.host.live(&actor) {
            return Ok(None);
        }
        let address = self.dispatcher.host.resolve_input_pointer(&source.pointer, words)?;
        if address == self.dispatcher.host.pointer(&actor, &source.record)? {
            Ok(Some(actor))
        } else {
            Ok(None)
        }
    }

    /// Open a client application, returning its token.
    pub fn open_application(&mut self, actor: ActorId) -> usize {
        self.applications.push(actor);
        self.applications.len() - 1
    }

    /// Close a client application by token.
    pub fn close_application(&mut self, token: usize) {
        if token < self.applications.len() {
            self.applications.remove(token);
        }
    }

    /// Push an input scope.
    pub fn push_input(&mut self, actor: ActorId) {
        self.inputs.push(StageInput {
            actor,
            scope: None,
            entered: false,
        });
    }

    /// Pop an input scope.
    pub fn pop_input(&mut self) {
        self.inputs.pop();
    }

    /// Run input code with an input scope pushed.
    pub fn apply(&mut self, actor: ActorId, run: impl FnOnce() -> i32) -> i32 {
        self.push_input(actor);
        let result = run();
        self.pop_input();
        result
    }

    /// Enter the innermost input scope, minting its cancellation scope.
    pub fn enter_input(&mut self, actor: &ActorId) -> BranchDecision {
        let Some(input) = self.inputs.last_mut() else {
            return BranchDecision::Take(true);
        };
        if input.entered {
            return BranchDecision::Take(true);
        }
        input.entered = true;
        let scope = self.dispatcher.host.fresh_scope();
        input.scope = Some(scope);
        if self.dispatcher.host.live(actor) {
            BranchDecision::Take(true)
        } else {
            BranchDecision::Cancel
        }
    }

    /// Cancel retired scopes of a dead actor.
    pub fn cancel_retired(&mut self, actor: &ActorId) {
        if self.dispatcher.host.live(actor) {
            return;
        }
        let scopes: Vec<u64> = self
            .inputs
            .iter()
            .filter(|input| input.actor == *actor)
            .filter_map(|input| input.scope)
            .collect();
        for scope in scopes {
            self.dispatcher.host.cancel_scope(scope);
        }
    }

    /// Cancellation scope for an actor: innermost input scope, else fresh.
    pub fn cancellation(&mut self, actor: &ActorId) -> u64 {
        for input in self.inputs.iter().rev() {
            if input.actor == *actor {
                if let Some(scope) = input.scope {
                    return scope;
                }
            }
        }
        self.dispatcher.host.fresh_scope()
    }

    /// Decide one continuation predicate.
    pub fn decide_continuation_predicate(
        &self,
        actor: &ActorId,
        predicate: &StagePredicate,
        original: bool,
    ) -> BranchDecision {
        if !self.dispatcher.host.live(actor) {
            return BranchDecision::Cancel;
        }
        BranchDecision::Take(if self.dispatcher.host.selected(actor) {
            original
        } else {
            predicate.unselected
        })
    }

    /// Decide the continuation boundary, reporting whether it continued.
    pub fn decide_continuation_boundary(&self, actor: &ActorId, original: bool) -> Result<BranchDecision, GuestError> {
        if !self.dispatcher.host.live(actor) {
            return Ok(BranchDecision::Cancel);
        }
        if original != self.definition.continuation.original_taken
            || !self.dispatcher.matches(actor, &self.definition.continuation.when)?
        {
            return Ok(BranchDecision::Take(original));
        }
        Ok(BranchDecision::Take(!original))
    }

    /// Whether a boundary decision continued into the projection path.
    #[must_use]
    pub fn continued_into(&self, original: bool, decision: BranchDecision) -> bool {
        decision == BranchDecision::Take(!original) && original == self.definition.continuation.original_taken
    }

    /// Finish a continuation: project posture and run ordered calls.
    pub fn finish_continuation(&mut self, actor: &ActorId, continued: bool, movement: usize) -> Result<(), GuestError> {
        if !continued {
            return Ok(());
        }
        let projection = self.definition.continuation.projection.clone();
        let posture = self.dispatcher.host.posture(actor)?;
        let host = &mut self.dispatcher.host;
        for (offset, value) in [
            (projection.minimum, posture.bounds_min),
            (projection.maximum, posture.bounds_max),
        ] {
            host.write_f32(movement + offset, value.x)?;
            host.write_f32(movement + offset + 4, value.y)?;
            host.write_f32(movement + offset + 8, value.z)?;
        }
        let view_height = host.pointer(actor, &projection.view_height.record)? + projection.view_height.offset;
        host.write_i32(view_height, posture.view_height)?;
        let ground = host.pointer(actor, &projection.ground.record)? + projection.ground.offset;
        host.write_i32(ground, posture.ground)?;
        for value in self.definition.continuation.calls.clone() {
            if !self.dispatcher.host.live(actor) {
                return Err(GuestError::invalid("QVM weapon continuation lost its source actor"));
            }
            self.dispatcher.host.invoke_source(actor, &value.call)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    use super::super::mod_provider::InputPointerKind;
    use super::*;

    struct FakeHost {
        words: HashMap<usize, i32>,
        floats: HashMap<usize, f32>,
        live: HashSet<ActorId>,
        selected: bool,
        attempted: Vec<(ActorId, i32)>,
        accepted: Vec<(ActorId, i32)>,
        completed: Vec<(ActorId, bool)>,
        invoked: Vec<usize>,
        cancelled: Vec<u64>,
        next_scope: u64,
    }

    impl FakeHost {
        fn new() -> Self {
            Self {
                words: HashMap::new(),
                floats: HashMap::new(),
                live: HashSet::new(),
                selected: true,
                attempted: Vec::new(),
                accepted: Vec::new(),
                completed: Vec::new(),
                invoked: Vec::new(),
                cancelled: Vec::new(),
                next_scope: 7,
            }
        }
    }

    impl WeaponStageHost for FakeHost {
        fn pointer(&self, _actor: &ActorId, _record: &str) -> Result<usize, GuestError> {
            Ok(100)
        }
        fn live(&self, actor: &ActorId) -> bool {
            self.live.contains(actor)
        }
        fn selected(&self, _actor: &ActorId) -> bool {
            self.selected
        }
        fn read_i32(&self, address: usize) -> Result<i32, GuestError> {
            Ok(self.words.get(&address).copied().unwrap_or(0))
        }
        fn write_i32(&mut self, address: usize, value: i32) -> Result<(), GuestError> {
            self.words.insert(address, value);
            Ok(())
        }
        fn write_f32(&mut self, address: usize, value: f32) -> Result<(), GuestError> {
            self.floats.insert(address, value);
            Ok(())
        }
        fn attempted(&mut self, actor: &ActorId, value: i32) {
            self.attempted.push((actor.clone(), value));
        }
        fn accepted(&mut self, actor: &ActorId, value: i32) {
            self.accepted.push((actor.clone(), value));
        }
        fn completed(&mut self, actor: &ActorId, reached_attack_decision: bool) {
            self.completed.push((actor.clone(), reached_attack_decision));
        }
        fn evaluate_region(&mut self, _region: &QvmRegionEvaluation, _inputs: &[i32]) -> Result<i32, GuestError> {
            Ok(3)
        }
        fn call_dispatcher(&mut self, _entry: usize) -> Result<i32, GuestError> {
            Ok(11)
        }
        fn resolve_input_pointer(&self, _pointer: &QvmModInputPointer, _words: &[i32]) -> Result<usize, GuestError> {
            Ok(100)
        }
        fn posture(&self, _actor: &ActorId) -> Result<WeaponPosture, GuestError> {
            Ok(WeaponPosture {
                bounds_min: vec3(-16.0, -16.0, -24.0),
                bounds_max: vec3(16.0, 16.0, 32.0),
                view_height: 26,
                ground: 1023,
            })
        }
        fn invoke_source(&mut self, _actor: &ActorId, call: &QvmModSourceCall) -> Result<i32, GuestError> {
            self.invoked.push(call.entry);
            Ok(0)
        }
        fn fresh_scope(&mut self) -> u64 {
            self.next_scope += 1;
            self.next_scope
        }
        fn cancel_scope(&mut self, scope: u64) {
            self.cancelled.push(scope);
        }
    }

    fn test_field(offset: usize) -> QvmItemField {
        QvmItemField {
            record: "client".to_string(),
            offset,
        }
    }

    fn test_pointer() -> QvmModInputPointer {
        QvmModInputPointer {
            kind: InputPointerKind::Argument { index: 0 },
            indirections: Vec::new(),
            offset: 0,
        }
    }

    fn fixture_definition() -> QvmWeaponDispatcherDefinition {
        QvmWeaponDispatcherDefinition {
            dispatcher: DispatcherHead {
                entry: 10,
                actor: QvmWeaponActor {
                    record: "client".to_string(),
                    pointer: test_pointer(),
                },
            },
            predicates: vec![StagePredicate {
                instruction: 11,
                unselected: true,
            }],
            settled: vec![QvmItemTest {
                field: test_field(0),
                mask: None,
                comparison: TestComparison::Equals,
                value: 3,
            }],
            selection: StageSelection {
                field: test_field(4),
                values: vec![
                    SelectionValue {
                        value: 1,
                        item: "test:mg".to_string(),
                    },
                    SelectionValue {
                        value: 2,
                        item: "test:sg".to_string(),
                    },
                ],
            },
            request: StageRequest {
                entry: 20,
                argument: 0,
                accepted: vec![QvmItemTest {
                    field: test_field(0),
                    mask: None,
                    comparison: TestComparison::Equals,
                    value: 3,
                }],
            },
        }
    }

    #[test]
    fn dispatcher_decides_predicates_and_settlement() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let dead = owner.actor(9, 1);
        let mut host = FakeHost::new();
        host.live.insert(actor.clone());
        host.words.insert(100, 3);
        host.words.insert(104, 1);
        let mut dispatcher = QvmWeaponDispatcher::new(fixture_definition(), host);
        assert!(dispatcher.settled(&actor).unwrap());
        assert_eq!(dispatcher.active(&actor).unwrap().as_deref(), Some("test:mg"));
        assert_eq!(
            dispatcher.decide_predicate(
                &actor,
                &StagePredicate {
                    instruction: 11,
                    unselected: false
                },
                true
            ),
            BranchDecision::Take(true)
        );
        assert_eq!(
            dispatcher.decide_predicate(
                &dead,
                &StagePredicate {
                    instruction: 11,
                    unselected: false
                },
                true
            ),
            BranchDecision::Cancel
        );
        dispatcher.begin_dispatch(actor.clone());
        dispatcher.decide_predicate(
            &actor,
            &StagePredicate {
                instruction: 11,
                unselected: false,
            },
            false,
        );
        dispatcher.end_dispatch(&actor);
        assert_eq!(dispatcher.host().completed, vec![(actor.clone(), true)]);
        assert!(dispatcher.request_gate(&actor, 2).unwrap());
        dispatcher.host_mut().words.insert(100, 0);
        assert!(!dispatcher.request_gate(&actor, 5).unwrap());
        assert_eq!(dispatcher.host().attempted, vec![(actor.clone(), 5)]);
        dispatcher.host_mut().words.insert(100, 3);
        dispatcher.request_finish(&actor, 5, false).unwrap();
        assert_eq!(dispatcher.host().accepted, vec![(actor.clone(), 5)]);
        let region = QvmRegionEvaluation {
            entry: 11,
            join: 12,
            inputs: vec![8],
            result: None,
        };
        assert_eq!(dispatcher.evaluate(&actor, &region, &[1, 2]).unwrap(), 11);
        assert!(dispatcher.evaluate(&dead, &region, &[1, 2]).is_err());
        assert_eq!(dispatcher.enter_evaluation(&actor).unwrap(), None);
    }

    #[test]
    fn stage_validation_accepts_original_layout() {
        let image = QvmImage {
            instructions: vec![
                QvmInstruction::word(QvmOpcode::OpEnter, 64),
                QvmInstruction::word(QvmOpcode::OpEq, 0),
                QvmInstruction::word(QvmOpcode::OpLeave, 0),
                QvmInstruction::word(QvmOpcode::OpEnter, 0),
                QvmInstruction::word(QvmOpcode::OpEnter, 48),
                QvmInstruction::word(QvmOpcode::OpNe, 0),
                QvmInstruction::word(QvmOpcode::OpLeave, 0),
                QvmInstruction::word(QvmOpcode::OpConst, 0),
                QvmInstruction::word(QvmOpcode::OpCall, 0),
                QvmInstruction::word(QvmOpcode::OpEnter, 0),
            ],
            data_length: 4096,
            literal_length: 0,
            bss_length: 0,
            initialized_length: 4096,
            allocated_data_length: 8192,
        };
        let settled = || QvmItemTest {
            field: test_field(0),
            mask: None,
            comparison: TestComparison::Equals,
            value: 1,
        };
        let stage = QvmWeaponStage {
            dispatcher: DispatcherHead {
                entry: 0,
                actor: QvmWeaponActor {
                    record: "client".to_string(),
                    pointer: test_pointer(),
                },
            },
            predicates: vec![StagePredicate {
                instruction: 1,
                unselected: false,
            }],
            settled: vec![settled()],
            selection: StageSelection {
                field: test_field(4),
                values: vec![SelectionValue {
                    value: 1,
                    item: "test:mg".to_string(),
                }],
            },
            request: StageRequest {
                entry: 3,
                argument: 0,
                accepted: vec![settled()],
            },
            continuation: StageContinuation {
                entry: 4,
                actor: QvmWeaponActor {
                    record: "client".to_string(),
                    pointer: test_pointer(),
                },
                instruction: 5,
                original_taken: true,
                when: vec![settled()],
                predicates: Vec::new(),
                projection: StageProjection {
                    movement: test_pointer(),
                    byte_length: 64,
                    minimum: 0,
                    maximum: 12,
                    view_height: test_field(16),
                    ground: test_field(20),
                },
                calls: vec![ContinuationCall {
                    instruction: 8,
                    call: QvmModSourceCall {
                        entry: 0,
                        arguments: Vec::new(),
                        globals: Vec::new(),
                        returns: ModReturns::Void,
                    },
                }],
            },
        };
        validate_qvm_weapon_stage(&stage, &image).unwrap();
    }

    #[test]
    fn stage_validation_rejects_broken_edges() {
        let image = QvmImage {
            instructions: vec![
                QvmInstruction::word(QvmOpcode::OpEnter, 8),
                QvmInstruction::word(QvmOpcode::OpLeave, 0),
            ],
            data_length: 64,
            literal_length: 0,
            bss_length: 0,
            initialized_length: 64,
            allocated_data_length: 128,
        };
        let mut definition = fixture_definition();
        definition.dispatcher.entry = 0;
        definition.predicates = vec![StagePredicate {
            instruction: 99,
            unselected: false,
        }];
        assert!(validate_qvm_weapon_dispatcher(&definition, &image).is_err());
        let mut stage = QvmWeaponStage {
            dispatcher: definition.dispatcher.clone(),
            predicates: definition.predicates.clone(),
            settled: definition.settled.clone(),
            selection: definition.selection.clone(),
            request: definition.request.clone(),
            continuation: StageContinuation {
                entry: 0,
                actor: QvmWeaponActor {
                    record: "client".to_string(),
                    pointer: test_pointer(),
                },
                instruction: 1,
                original_taken: false,
                when: Vec::new(),
                predicates: Vec::new(),
                projection: StageProjection {
                    movement: test_pointer(),
                    byte_length: 64,
                    minimum: 0,
                    maximum: 12,
                    view_height: test_field(16),
                    ground: test_field(20),
                },
                calls: Vec::new(),
            },
        };
        assert!(validate_qvm_weapon_stage(&stage, &image).is_err());
        stage.predicates = vec![StagePredicate {
            instruction: 1,
            unselected: false,
        }];
        assert!(validate_qvm_weapon_stage(&stage, &image).is_err());
    }

    #[test]
    fn continuation_projects_posture_and_runs_calls() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let dead = owner.actor(9, 1);
        let mut host = FakeHost::new();
        host.live.insert(actor.clone());
        let mut definition = fixture_definition();
        let stage = QvmWeaponStage {
            dispatcher: definition.dispatcher.clone(),
            predicates: definition.predicates.clone(),
            settled: definition.settled.clone(),
            selection: definition.selection.clone(),
            request: std::mem::replace(
                &mut definition.request,
                StageRequest {
                    entry: 0,
                    argument: 0,
                    accepted: Vec::new(),
                },
            ),
            continuation: StageContinuation {
                entry: 30,
                actor: QvmWeaponActor {
                    record: "client".to_string(),
                    pointer: test_pointer(),
                },
                instruction: 31,
                original_taken: false,
                when: vec![QvmItemTest {
                    field: test_field(0),
                    mask: None,
                    comparison: TestComparison::Equals,
                    value: 3,
                }],
                predicates: Vec::new(),
                projection: StageProjection {
                    movement: test_pointer(),
                    byte_length: 64,
                    minimum: 0,
                    maximum: 12,
                    view_height: test_field(16),
                    ground: test_field(20),
                },
                calls: vec![ContinuationCall {
                    instruction: 40,
                    call: QvmModSourceCall {
                        entry: 10,
                        arguments: Vec::new(),
                        globals: Vec::new(),
                        returns: ModReturns::Void,
                    },
                }],
            },
        };
        let mut stage = QvmModWeaponStage::new(stage, host);
        let token = stage.open_application(actor.clone());
        assert_eq!(
            stage
                .actor_for(
                    &QvmWeaponActor {
                        record: "client".to_string(),
                        pointer: test_pointer()
                    },
                    &[100]
                )
                .unwrap(),
            Some(actor.clone())
        );
        stage.close_application(token);
        assert_eq!(
            stage
                .actor_for(
                    &QvmWeaponActor {
                        record: "client".to_string(),
                        pointer: test_pointer()
                    },
                    &[100]
                )
                .unwrap(),
            None
        );
        let decided = stage.decide_continuation_boundary(&actor, true).unwrap();
        assert_eq!(decided, BranchDecision::Take(true));
        assert!(stage.continued_into(false, BranchDecision::Take(true)));
        stage.finish_continuation(&actor, true, 500).unwrap();
        assert_eq!(stage.dispatcher_mut().host_mut().invoked, vec![10]);
        assert_eq!(stage.dispatcher().host().words.get(&116), Some(&26));
        assert_eq!(stage.dispatcher().host().words.get(&120), Some(&1023));
        let result = stage.apply(actor.clone(), || 5);
        assert_eq!(result, 5);
        stage.push_input(dead.clone());
        assert!(matches!(stage.enter_input(&dead), BranchDecision::Cancel));
        stage.pop_input();
        stage.cancel_retired(&dead);
    }
}
