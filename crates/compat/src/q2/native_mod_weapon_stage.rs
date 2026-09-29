//! Port of `src/compat/q2/native-mod-weapon-stage.ts`.
//! Bridges the original weapon dispatcher: firing, latch, animation, and
//! callback timing stay guest-owned while input projection and selection
//! compose on the shared boundary.

use std::collections::{HashMap, HashSet};

use qa_guest::core::contracts::{GuestAddress, GuestStorage};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::error::GuestError;
use thiserror::Error;

/// Failures in the native weapon stage.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum WeaponError {
    /// Weapon input outlived its source actor.
    #[error("native weapon input outlived its source actor")]
    ActorGone,
    /// The source selected an undeclared weapon.
    #[error("original native selected an undeclared source weapon")]
    UndeclaredWeapon,
    /// A dispatcher declaration is invalid.
    #[error("native weapon dispatcher lacks its actor ABI")]
    BadDispatcher,
    /// A decision region is invalid.
    #[error("invalid native weapon input-read region")]
    BadRegion,
    /// A named export is not declared.
    #[error("native weapon export is not declared: {0}")]
    MissingExport(String),
    /// A record base is not bound.
    #[error("native weapon record is not bound: {0}")]
    MissingRecord(String),
    /// A scalar test value is not usable.
    #[error("native weapon test value is invalid")]
    BadTestValue,
    /// A cancelled dispatch failed outside cancellation.
    #[error("native weapon dispatch failed: {0}")]
    Dispatch(String),
    /// Underlying guest failure.
    #[error("native weapon guest failure: {0}")]
    Guest(String),
}

impl From<GuestError> for WeaponError {
    fn from(error: GuestError) -> Self {
        Self::Guest(error.to_string())
    }
}

/// Generational actor handle local to the weapon bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeActorId {
    /// Registry slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
}

/// Scalar item field inside a record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemField {
    /// Owning record.
    pub record: String,
    /// Byte offset.
    pub offset: usize,
    /// Lane storage.
    pub storage: GuestStorage,
}

/// Pointer field inside a record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemPointer {
    /// Owning record.
    pub record: String,
    /// Byte offset.
    pub offset: usize,
}

/// Address reference: export name or image RVA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressRef {
    /// Named export.
    Export(String),
    /// Image-relative offset.
    Rva(u64),
}

/// Scalar test comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestComparison {
    /// Exact equality.
    Equals,
    /// Less than or equal.
    AtMost,
}

/// One weapon state test.
#[derive(Debug, Clone, PartialEq)]
pub enum ItemTest {
    /// Scalar comparison, optionally masked.
    Scalar {
        /// Tested field.
        field: ItemField,
        /// Comparison.
        comparison: TestComparison,
        /// Compared value.
        value: f64,
        /// Bitmask applied before comparison, if any.
        mask: Option<i64>,
    },
    /// Pointer identity against a declared address.
    Pointer {
        /// Tested field.
        field: ItemPointer,
        /// Expected address, if any.
        value: Option<AddressRef>,
    },
}

/// Dispatcher declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatcherDecl {
    /// Dispatcher entry.
    pub entry: AddressRef,
    /// Actor record for the dispatcher argument.
    pub record: String,
    /// Argument count.
    pub arguments: usize,
    /// Actor argument index.
    pub argument: usize,
}

/// One input projection inside a decision region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionField {
    /// Projected field.
    pub field: ItemField,
    /// Bits cleared while the region runs.
    pub clear_mask: u32,
}

/// Decision region declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionRegion {
    /// Region entry RVA.
    pub entry: u64,
    /// Region join RVA.
    pub join: u64,
    /// Projected fields.
    pub fields: Vec<DecisionField>,
}

/// Declared source call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponCall {
    /// Call id.
    pub id: String,
}

/// One selectable weapon value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionValue {
    /// Canonical item.
    pub item: String,
    /// Source weapon address.
    pub address: AddressRef,
    /// Selection request call.
    pub request: WeaponCall,
}

/// Selection pointer declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionDecl {
    /// Active weapon pointer.
    pub active: ItemPointer,
    /// Pending weapon pointer, if any.
    pub pending: Option<ItemPointer>,
    /// Declared values.
    pub values: Vec<SelectionValue>,
}

/// Weapon stage declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponStageDefinition {
    /// Dispatcher declaration.
    pub dispatcher: DispatcherDecl,
    /// Decision regions.
    pub decisions: Vec<DecisionRegion>,
    /// Settled test groups (any group matches).
    pub settled: Vec<Vec<ItemTest>>,
    /// Continuation test groups (any group matches).
    pub continuations: Vec<Vec<ItemTest>>,
    /// Committed-input test groups, if any.
    pub committed_input: Option<Vec<Vec<ItemTest>>>,
    /// Selection declaration.
    pub selection: SelectionDecl,
}

/// Weapon operations: actor resolution plus source scalar access.
pub trait NativeWeaponOperations {
    /// Actor owning a record address, if bound.
    fn actor_for(
        &mut self,
        record: &str,
        address: GuestAddress,
    ) -> Option<NativeActorId>;
    /// Whether an actor is current.
    fn is_current(&self, actor: NativeActorId) -> bool;
    /// Whether an actor holds weapon selection.
    fn is_selected(&self, actor: NativeActorId) -> bool;
    /// Pointer field address for an actor.
    fn pointer(
        &mut self,
        actor: NativeActorId,
        field: &ItemPointer,
    ) -> Result<GuestAddress, WeaponError>;
    /// Resolve a declared address.
    fn resolve(&self, address: &AddressRef) -> Result<GuestAddress, WeaponError>;
    /// Read a scalar field for an actor.
    fn read(&mut self, actor: NativeActorId, field: &ItemField) -> Result<f64, WeaponError>;
    /// Write a scalar field for an actor.
    fn write(
        &mut self,
        actor: NativeActorId,
        field: &ItemField,
        value: f64,
    ) -> Result<(), WeaponError>;
    /// Read a pointer value.
    fn read_pointer(
        &mut self,
        address: GuestAddress,
    ) -> Result<Option<GuestAddress>, WeaponError>;
    /// Invoke a source call for an actor.
    fn invoke(&mut self, actor: NativeActorId, call: &WeaponCall);
    /// Record dispatch completion.
    fn completed(&mut self, actor: NativeActorId, reached_decision: bool);
    /// Capture the synthetic processor token.
    fn state_token(&self) -> u64;
    /// Restore a captured processor token.
    fn restore_token(&mut self, token: u64);
    /// Whether a dispatch error is an accepted cancellation.
    fn cancellation_accepts(&self, actor: NativeActorId) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DispatchFrame {
    actor: NativeActorId,
    committed_input: bool,
    reached_decision: bool,
}

/// Original dispatcher bridge: frames plus decision-region projection.
#[derive(Debug, Default)]
pub struct NativeWeaponDispatcher {
    frames: Vec<DispatchFrame>,
}

impl NativeWeaponDispatcher {
    /// Build the dispatcher, validating the actor ABI declaration.
    pub fn new(definition: &WeaponStageDefinition) -> Result<Self, WeaponError> {
        let dispatcher = &definition.dispatcher;
        if dispatcher.arguments < 1
            || dispatcher.arguments > 16
            || dispatcher.argument >= dispatcher.arguments
        {
            return Err(WeaponError::BadDispatcher);
        }
        for region in &definition.decisions {
            if region.join <= region.entry || region.fields.is_empty() {
                return Err(WeaponError::BadRegion);
            }
        }
        Ok(Self { frames: Vec::new() })
    }

    /// Actor of the innermost dispatch frame, if any.
    #[must_use]
    pub fn current_actor(&self) -> Option<NativeActorId> {
        self.frames.last().map(|frame| frame.actor)
    }

    /// Open a dispatch frame for an actor.
    pub fn open_frame(&mut self, actor: NativeActorId, committed_input: bool) {
        self.frames.push(DispatchFrame {
            actor,
            committed_input,
            reached_decision: false,
        });
    }

    /// Close the innermost frame, recording completion when current.
    pub fn close_frame<O: NativeWeaponOperations>(&mut self, operations: &mut O) {
        if let Some(frame) = self.frames.pop() {
            if operations.is_current(frame.actor) {
                operations.completed(frame.actor, frame.reached_decision);
            }
        }
    }

    /// Drop the innermost frame without recording completion.
    pub fn abort_frame(&mut self) {
        self.frames.pop();
    }

    /// Run the dispatcher body for a record address.
    pub fn dispatch<R, O: NativeWeaponOperations>(
        &mut self,
        operations: &mut O,
        record: &str,
        address: Option<GuestAddress>,
        committed_input: impl Fn(NativeActorId) -> bool,
        body: impl FnOnce(&mut O) -> Result<R, WeaponError>,
    ) -> Result<Option<R>, WeaponError> {
        let Some(address) = address else {
            return Ok(Some(body(operations)?));
        };
        let Some(actor) = operations.actor_for(record, address) else {
            return Ok(Some(body(operations)?));
        };
        self.dispatch_actor(operations, actor, committed_input(actor), body)
    }

    /// Run the dispatcher body for a resolved actor.
    pub fn dispatch_actor<R, O: NativeWeaponOperations>(
        &mut self,
        operations: &mut O,
        actor: NativeActorId,
        committed_input: bool,
        body: impl FnOnce(&mut O) -> Result<R, WeaponError>,
    ) -> Result<Option<R>, WeaponError> {
        let token = operations.state_token();
        self.open_frame(actor, committed_input);
        let outcome = body(operations);
        match outcome {
            Ok(value) => {
                self.close_frame(operations);
                Ok(Some(value))
            }
            Err(error) => {
                self.abort_frame();
                if operations.cancellation_accepts(actor) && operations.is_current(actor) {
                    operations.restore_token(token);
                    Ok(None)
                } else {
                    Err(error)
                }
            }
        }
    }

    /// Run a decision region with input projection for unselected actors.
    pub fn run_decision<R, O: NativeWeaponOperations>(
        &mut self,
        operations: &mut O,
        region: &DecisionRegion,
        body: impl FnOnce(&mut O) -> R,
    ) -> Result<R, WeaponError> {
        let actor = match self.frames.last_mut() {
            None => return Ok(body(operations)),
            Some(frame) => {
                frame.reached_decision = true;
                if frame.committed_input {
                    return Ok(body(operations));
                }
                frame.actor
            }
        };
        if operations.is_selected(actor) {
            return Ok(body(operations));
        }
        if !operations.is_current(actor) {
            return Err(WeaponError::ActorGone);
        }
        let mut projected = Vec::with_capacity(region.fields.len());
        for value in &region.fields {
            projected.push((value, operations.read(actor, &value.field)?));
        }
        for (value, original) in &projected {
            let masked = (*original as i64) & !(value.clear_mask as i64);
            operations.write(actor, &value.field, masked as f64)?;
        }
        let result = body(operations);
        // Regions contain source reads, never source writes; preserve
        // unrelated bits defensively.
        for (value, original) in &projected {
            let current = operations.read(actor, &value.field)?;
            let restored = (current as i64 & !(value.clear_mask as i64))
                | (*original as i64 & value.clear_mask as i64);
            operations.write(actor, &value.field, restored as f64)?;
        }
        Ok(result)
    }

    /// Drop all frames.
    pub fn close(&mut self) {
        self.frames.clear();
    }
}

/// Memory-backed weapon operations over synthetic record rows.
pub struct SyntheticWeaponOperations {
    /// Guest memory.
    pub memory: SparseGuestMemory,
    image_base: GuestAddress,
    record_bases: HashMap<(String, u32), GuestAddress>,
    exports: HashMap<String, GuestAddress>,
    current: HashSet<NativeActorId>,
    selected: HashSet<NativeActorId>,
    cancellations: HashSet<NativeActorId>,
    state: u64,
    /// Invoked source calls.
    pub invokes: Vec<(NativeActorId, String)>,
    /// Completion records.
    pub completions: Vec<(NativeActorId, bool)>,
}

impl SyntheticWeaponOperations {
    /// Build operations over guest memory.
    pub fn new(memory: SparseGuestMemory, image_base: GuestAddress) -> Self {
        Self {
            memory,
            image_base,
            record_bases: HashMap::new(),
            exports: HashMap::new(),
            current: HashSet::new(),
            selected: HashSet::new(),
            cancellations: HashSet::new(),
            state: 0,
            invokes: Vec::new(),
            completions: Vec::new(),
        }
    }

    /// Bind a record base for an actor slot.
    pub fn bind_record(&mut self, actor: NativeActorId, record: &str, base: GuestAddress) {
        self.record_bases.insert((record.to_string(), actor.slot), base);
    }

    /// Register a named export.
    pub fn register_export(&mut self, name: &str, address: GuestAddress) {
        self.exports.insert(name.to_string(), address);
    }

    /// Mark an actor current.
    pub fn set_current(&mut self, actor: NativeActorId, current: bool) {
        if current {
            self.current.insert(actor);
        } else {
            self.current.remove(&actor);
        }
    }

    /// Mark an actor selected.
    pub fn set_selected(&mut self, actor: NativeActorId, selected: bool) {
        if selected {
            self.selected.insert(actor);
        } else {
            self.selected.remove(&actor);
        }
    }

    /// Accept cancellations for an actor.
    pub fn set_cancellation(&mut self, actor: NativeActorId, accepts: bool) {
        if accepts {
            self.cancellations.insert(actor);
        } else {
            self.cancellations.remove(&actor);
        }
    }

    /// Overwrite the synthetic processor token.
    pub fn set_state(&mut self, state: u64) {
        self.state = state;
    }

    fn base(&self, actor: NativeActorId, record: &str) -> Result<GuestAddress, WeaponError> {
        self.record_bases
            .get(&(record.to_string(), actor.slot))
            .copied()
            .ok_or_else(|| WeaponError::MissingRecord(record.to_string()))
    }
}

fn scalar_to_f64(bytes: &[u8], storage: GuestStorage) -> f64 {
    match storage {
        GuestStorage::Float32 => {
            f64::from(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        }
        GuestStorage::Float64 => {
            let mut word = [0u8; 8];
            word.copy_from_slice(&bytes[..8]);
            f64::from_le_bytes(word)
        }
        GuestStorage::Int8 => f64::from(bytes[0] as i8),
        GuestStorage::Uint8 => f64::from(bytes[0]),
        GuestStorage::Int16 => f64::from(i16::from_le_bytes([bytes[0], bytes[1]])),
        GuestStorage::Uint16 => f64::from(u16::from_le_bytes([bytes[0], bytes[1]])),
        GuestStorage::Int32 => {
            f64::from(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        }
        GuestStorage::Uint32 => {
            f64::from(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        }
        GuestStorage::Int64 => {
            let mut word = [0u8; 8];
            word.copy_from_slice(&bytes[..8]);
            i64::from_le_bytes(word) as f64
        }
        GuestStorage::Uint64 => {
            let mut word = [0u8; 8];
            word.copy_from_slice(&bytes[..8]);
            u64::from_le_bytes(word) as f64
        }
        GuestStorage::Pointer => {
            let mut word = [0u8; 8];
            word.copy_from_slice(&bytes[..bytes.len().min(8)]);
            u64::from_le_bytes(word) as f64
        }
    }
}

fn f64_to_scalar(value: f64, storage: GuestStorage) -> Vec<u8> {
    match storage {
        GuestStorage::Float32 => (value as f32).to_le_bytes().to_vec(),
        GuestStorage::Float64 => value.to_le_bytes().to_vec(),
        GuestStorage::Int8 | GuestStorage::Uint8 => vec![value as i64 as u8],
        GuestStorage::Int16 | GuestStorage::Uint16 => {
            (value as i64 as i16).to_le_bytes().to_vec()
        }
        GuestStorage::Int32 | GuestStorage::Uint32 | GuestStorage::Pointer => {
            (value as i64 as i32).to_le_bytes().to_vec()
        }
        GuestStorage::Int64 | GuestStorage::Uint64 => (value as i64).to_le_bytes().to_vec(),
    }
}

impl NativeWeaponOperations for SyntheticWeaponOperations {
    fn actor_for(
        &mut self,
        record: &str,
        address: GuestAddress,
    ) -> Option<NativeActorId> {
        self.record_bases.iter().find(|((bound, _), base)| {
            bound == record && **base == address
        }).map(|((_, slot), _)| NativeActorId {
            slot: *slot,
            generation: 1,
        })
    }

    fn is_current(&self, actor: NativeActorId) -> bool {
        self.current.contains(&actor)
    }

    fn is_selected(&self, actor: NativeActorId) -> bool {
        self.selected.contains(&actor)
    }

    fn pointer(
        &mut self,
        actor: NativeActorId,
        field: &ItemPointer,
    ) -> Result<GuestAddress, WeaponError> {
        let base = self.base(actor, &field.record)?;
        Ok(self.memory.offset(base, field.offset as i64)?)
    }

    fn resolve(&self, address: &AddressRef) -> Result<GuestAddress, WeaponError> {
        match address {
            AddressRef::Export(name) => self
                .exports
                .get(name)
                .copied()
                .ok_or_else(|| WeaponError::MissingExport(name.clone())),
            AddressRef::Rva(rva) => Ok(self
                .memory
                .offset(self.image_base, i64::try_from(*rva).unwrap_or(i64::MAX))?),
        }
    }

    fn read(&mut self, actor: NativeActorId, field: &ItemField) -> Result<f64, WeaponError> {
        let base = self.base(actor, &field.record)?;
        let address = self.memory.offset(base, field.offset as i64)?;
        let width = field.storage.byte_length(self.memory.pointer_bytes());
        Ok(scalar_to_f64(&self.memory.copy(address, width)?, field.storage))
    }

    fn write(
        &mut self,
        actor: NativeActorId,
        field: &ItemField,
        value: f64,
    ) -> Result<(), WeaponError> {
        let base = self.base(actor, &field.record)?;
        let address = self.memory.offset(base, field.offset as i64)?;
        Ok(self.memory.write(address, &f64_to_scalar(value, field.storage))?)
    }

    fn read_pointer(
        &mut self,
        address: GuestAddress,
    ) -> Result<Option<GuestAddress>, WeaponError> {
        Ok(self.memory.read_pointer(address)?)
    }

    fn invoke(&mut self, actor: NativeActorId, call: &WeaponCall) {
        self.invokes.push((actor, call.id.clone()));
    }

    fn completed(&mut self, actor: NativeActorId, reached_decision: bool) {
        self.completions.push((actor, reached_decision));
    }

    fn state_token(&self) -> u64 {
        self.state
    }

    fn restore_token(&mut self, token: u64) {
        self.state = token;
    }

    fn cancellation_accepts(&self, actor: NativeActorId) -> bool {
        self.cancellations.contains(&actor)
    }
}

/// Weapon stage: dispatcher plus selection and settled predicates.
pub struct NativeModWeaponStage<O: NativeWeaponOperations> {
    definition: WeaponStageDefinition,
    dispatcher: NativeWeaponDispatcher,
    operations: O,
}

impl<O: NativeWeaponOperations> NativeModWeaponStage<O> {
    /// Build the stage, validating the dispatcher declaration.
    pub fn new(definition: WeaponStageDefinition, operations: O) -> Result<Self, WeaponError> {
        let dispatcher = NativeWeaponDispatcher::new(&definition)?;
        Ok(Self {
            definition,
            dispatcher,
            operations,
        })
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

    /// Actor of the innermost dispatch frame, if any.
    #[must_use]
    pub fn current_actor(&self) -> Option<NativeActorId> {
        self.dispatcher.current_actor()
    }

    fn matches(&mut self, actor: NativeActorId, test: &ItemTest) -> Result<bool, WeaponError> {
        match test {
            ItemTest::Pointer { field, value } => {
                let address = self.operations.pointer(actor, field)?;
                let current = self.operations.read_pointer(address)?.map(|pointer| pointer.offset);
                let expected = value
                    .as_ref()
                    .map(|reference| self.operations.resolve(reference))
                    .transpose()?
                    .map(|pointer| pointer.offset);
                Ok(current == expected)
            }
            ItemTest::Scalar { field, comparison, value, mask } => {
                if !value.is_finite() {
                    return Err(WeaponError::BadTestValue);
                }
                let raw = self.operations.read(actor, field)?;
                let reduced = match mask {
                    None => raw,
                    Some(mask) => f64::from((raw as i64 & mask) as i32),
                };
                Ok(match comparison {
                    TestComparison::Equals => reduced == *value,
                    TestComparison::AtMost => reduced <= *value,
                })
            }
        }
    }

    /// Whether an actor sits inside a continuation group.
    pub fn continuing(&mut self, actor: NativeActorId) -> Result<bool, WeaponError> {
        if !self.operations.is_current(actor) {
            return Ok(false);
        }
        let groups = self.definition.continuations.clone();
        for tests in &groups {
            let mut matched = true;
            for test in tests {
                if !self.matches(actor, test)? {
                    matched = false;
                    break;
                }
            }
            if matched {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether an actor sits inside a settled group.
    pub fn settled(&mut self, actor: NativeActorId) -> Result<bool, WeaponError> {
        if !self.operations.is_current(actor) {
            return Ok(false);
        }
        let groups = self.definition.settled.clone();
        for tests in &groups {
            let mut matched = true;
            for test in tests {
                if !self.matches(actor, test)? {
                    matched = false;
                    break;
                }
            }
            if matched {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether an actor carries committed input.
    pub fn committed(&mut self, actor: NativeActorId) -> Result<bool, WeaponError> {
        if !self.operations.is_current(actor) {
            return Ok(false);
        }
        let Some(groups) = self.definition.committed_input.clone() else {
            return Ok(false);
        };
        for tests in &groups {
            let mut matched = true;
            for test in tests {
                if !self.matches(actor, test)? {
                    matched = false;
                    break;
                }
            }
            if matched {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn selected(
        &mut self,
        actor: NativeActorId,
        field: &ItemPointer,
    ) -> Result<Option<String>, WeaponError> {
        let address = self.operations.pointer(actor, field)?;
        let Some(current) = self.operations.read_pointer(address)? else {
            return Ok(None);
        };
        for value in self.definition.selection.values.clone() {
            if self.operations.resolve(&value.address)? == current {
                return Ok(Some(value.item));
            }
        }
        Err(WeaponError::UndeclaredWeapon)
    }

    /// Active weapon item, if any.
    pub fn active(&mut self, actor: NativeActorId) -> Result<Option<String>, WeaponError> {
        let field = self.definition.selection.active.clone();
        self.selected(actor, &field)
    }

    /// Pending weapon item, if any.
    pub fn pending(&mut self, actor: NativeActorId) -> Result<Option<String>, WeaponError> {
        let Some(field) = self.definition.selection.pending.clone() else {
            return Ok(None);
        };
        self.selected(actor, &field)
    }

    /// Request a weapon through its declared source call.
    pub fn request(&mut self, actor: NativeActorId, item: &str) -> Result<bool, WeaponError> {
        let Some(value) = self
            .definition
            .selection
            .values
            .iter()
            .find(|value| value.item == item)
            .cloned()
        else {
            return Ok(false);
        };
        if !self.operations.is_current(actor) {
            return Ok(false);
        }
        self.operations.invoke(actor, &value.request);
        Ok(self.operations.is_current(actor)
            && (self.active(actor)? == Some(item.to_string())
                || self.pending(actor)? == Some(item.to_string())))
    }

    /// Run the dispatcher body for a record address.
    pub fn dispatch<R>(
        &mut self,
        record: &str,
        address: Option<GuestAddress>,
        body: impl FnOnce(&mut O) -> Result<R, WeaponError>,
    ) -> Result<Option<R>, WeaponError> {
        let actor = match address {
            None => None,
            Some(address) => self.operations.actor_for(record, address),
        };
        let committed = match actor {
            None => false,
            Some(actor) => self.committed(actor)?,
        };
        // Re-resolve inside the dispatcher frame for one shared code path.
        let record = record.to_string();
        self.dispatcher.dispatch(
            &mut self.operations,
            &record,
            address,
            |_| committed,
            body,
        )
    }

    /// Run a decision region with input projection.
    pub fn run_decision<R>(
        &mut self,
        region: &DecisionRegion,
        body: impl FnOnce(&mut O) -> R,
    ) -> Result<R, WeaponError> {
        let region = region.clone();
        self.dispatcher.run_decision(&mut self.operations, &region, body)
    }

    /// Run a closure inside an explicit dispatch frame for an actor.
    pub fn run_in_frame<R>(
        &mut self,
        actor: NativeActorId,
        run: impl FnOnce(&mut NativeWeaponDispatcher, &mut O) -> Result<R, WeaponError>,
    ) -> Result<Option<R>, WeaponError> {
        let committed = self.committed(actor)?;
        let token = self.operations.state_token();
        self.dispatcher.open_frame(actor, committed);
        let outcome = run(&mut self.dispatcher, &mut self.operations);
        match outcome {
            Ok(value) => {
                self.dispatcher.close_frame(&mut self.operations);
                Ok(Some(value))
            }
            Err(error) => {
                self.dispatcher.abort_frame();
                if self.operations.cancellation_accepts(actor)
                    && self.operations.is_current(actor)
                {
                    self.operations.restore_token(token);
                    Ok(None)
                } else {
                    Err(error)
                }
            }
        }
    }

    /// Drop dispatcher frames.
    pub fn close(&mut self) {
        self.dispatcher.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{
        ContentDigest, GuestMapOptions, GuestPermissions, ModuleIdentity,
    };

    fn actor(slot: u32) -> NativeActorId {
        NativeActorId { slot, generation: 1 }
    }

    fn field(record: &str, offset: usize) -> ItemField {
        ItemField {
            record: record.to_string(),
            offset,
            storage: GuestStorage::Int32,
        }
    }

    fn pointer(record: &str, offset: usize) -> ItemPointer {
        ItemPointer {
            record: record.to_string(),
            offset,
        }
    }

    fn definition() -> WeaponStageDefinition {
        WeaponStageDefinition {
            dispatcher: DispatcherDecl {
                entry: AddressRef::Rva(0x800),
                record: "player".to_string(),
                arguments: 2,
                argument: 1,
            },
            decisions: vec![DecisionRegion {
                entry: 0x900,
                join: 0x980,
                fields: vec![DecisionField {
                    field: field("player", 0),
                    clear_mask: 0x2,
                }],
            }],
            settled: vec![vec![ItemTest::Scalar {
                field: field("player", 4),
                comparison: TestComparison::Equals,
                value: 1.0,
                mask: None,
            }]],
            continuations: vec![vec![ItemTest::Scalar {
                field: field("player", 4),
                comparison: TestComparison::Equals,
                value: 2.0,
                mask: None,
            }]],
            committed_input: None,
            selection: SelectionDecl {
                active: pointer("player", 8),
                pending: Some(pointer("player", 12)),
                values: vec![
                    SelectionValue {
                        item: "q2:blaster".to_string(),
                        address: AddressRef::Rva(0xA00),
                        request: WeaponCall { id: "use-blaster".to_string() },
                    },
                    SelectionValue {
                        item: "q2:shotgun".to_string(),
                        address: AddressRef::Rva(0xA40),
                        request: WeaponCall { id: "use-shotgun".to_string() },
                    },
                ],
            },
        }
    }

    fn fixture() -> NativeModWeaponStage<SyntheticWeaponOperations> {
        let module = ModuleIdentity::new(
            ProviderId::new("test", "weapon"),
            "weapon.so",
            ContentDigest {
                algorithm: "none".to_string(),
                value: "0".to_string(),
            },
            "r1",
        );
        let mut memory = SparseGuestMemory::new(module, 4, 0xC0000).unwrap();
        let image_base = memory
            .map(&GuestMapOptions::new(0x0, 0x2000, GuestPermissions::ReadWrite))
            .unwrap();
        let row = memory
            .map(&GuestMapOptions::new(0x30000, 64, GuestPermissions::ReadWrite))
            .unwrap();
        let mut operations = SyntheticWeaponOperations::new(memory, image_base);
        operations.bind_record(actor(1), "player", row);
        operations.set_current(actor(1), true);
        NativeModWeaponStage::new(definition(), operations).unwrap()
    }

    #[test]
    fn decision_masks_input_for_unselected_actors_only() {
        let mut stage = fixture();
        stage.operations_mut().write(actor(1), &field("player", 0), 3.0).unwrap();
        // An address that names no record base runs guest-direct, frameless.
        let field_address = stage.operations_mut().pointer(actor(1), &pointer("player", 0)).unwrap();
        let direct = stage
            .dispatch("player", Some(field_address), |_| Ok::<_, WeaponError>(0u32))
            .unwrap();
        assert_eq!(direct, Some(0));
        assert!(stage.operations().completions.is_empty());
        assert_eq!(stage.current_actor(), None);

        let region = definition().decisions[0].clone();
        // Unselected actor: the masked bit clears inside the region, then restores.
        let mut seen = 0.0;
        stage
            .run_in_frame(actor(1), |dispatcher, operations| {
                dispatcher.run_decision(operations, &region, |operations| {
                    seen = operations.read(actor(1), &field("player", 0)).unwrap();
                })
            })
            .unwrap();
        assert_eq!(seen, 1.0);
        assert_eq!(
            stage.operations_mut().read(actor(1), &field("player", 0)).unwrap(),
            3.0
        );
        assert_eq!(stage.operations().completions, vec![(actor(1), true)]);

        // Selected actor: no projection is applied.
        stage.operations_mut().set_selected(actor(1), true);
        let mut seen_selected = 0.0;
        stage
            .run_in_frame(actor(1), |dispatcher, operations| {
                dispatcher.run_decision(operations, &region, |operations| {
                    seen_selected = operations.read(actor(1), &field("player", 0)).unwrap();
                })
            })
            .unwrap();
        assert_eq!(seen_selected, 3.0);
    }

    #[test]
    fn selection_request_and_predicates() {
        let mut stage = fixture();
        assert_eq!(stage.active(actor(1)).unwrap(), None);
        assert_eq!(stage.pending(actor(1)).unwrap(), None);
        assert!(!stage.settled(actor(1)).unwrap());
        stage.operations_mut().write(actor(1), &field("player", 4), 1.0).unwrap();
        assert!(stage.settled(actor(1)).unwrap());
        assert!(!stage.continuing(actor(1)).unwrap());

        // Point the active slot at the blaster and request it.
        let blaster = stage.operations().resolve(&AddressRef::Rva(0xA00)).unwrap();
        let active = stage.operations_mut().pointer(actor(1), &pointer("player", 8)).unwrap();
        stage.operations_mut().memory.write_pointer(active, Some(blaster)).unwrap();
        assert_eq!(stage.active(actor(1)).unwrap(), Some("q2:blaster".to_string()));
        assert!(stage.request(actor(1), "q2:blaster").unwrap());
        assert_eq!(
            stage.operations().invokes,
            vec![(actor(1), "use-blaster".to_string())]
        );
        assert!(!stage.request(actor(1), "q2:railgun").unwrap());

        let shotgun = stage.operations().resolve(&AddressRef::Rva(0xA40)).unwrap();
        let other = GuestAddress::new(shotgun.space, shotgun.offset + 4);
        stage.operations_mut().memory.write_pointer(active, Some(other)).unwrap();
        assert_eq!(
            stage.active(actor(1)),
            Err(WeaponError::UndeclaredWeapon)
        );
    }

    #[test]
    fn cancellation_restores_state_and_foreign_errors_propagate() {
        let mut stage = fixture();
        stage.operations_mut().set_cancellation(actor(1), true);
        stage.operations_mut().set_state(0xAAAA);
        let base = stage.operations().record_bases[&("player".to_string(), 1)];
        let cancelled = stage
            .dispatch("player", Some(base), |operations| {
                operations.set_state(0xBBBB);
                Err::<(), _>(WeaponError::Dispatch("retired".to_string()))
            })
            .unwrap();
        assert_eq!(cancelled, None);
        assert_eq!(stage.operations().state_token(), 0xAAAA);

        stage.operations_mut().set_cancellation(actor(1), false);
        let failed = stage.dispatch("player", Some(base), |_| {
            Err::<(), _>(WeaponError::Dispatch("boom".to_string()))
        });
        assert_eq!(
            failed,
            Err(WeaponError::Dispatch("boom".to_string()))
        );
        stage.close();
    }
}
