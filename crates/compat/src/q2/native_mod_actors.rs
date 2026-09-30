//! Port of `src/compat/q2/native-mod-actors.ts`.
//! Bridges source-owned actors: private words stay guest-owned while adoption,
//! the source clock, and the update loop compose on the shared boundary.

use std::collections::{HashMap, HashSet};

use qa_core::math::Vec3;
use qa_guest::core::contracts::{GuestAccess, GuestAddress, GuestCallResult, GuestCallValue, GuestStorage};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::error::GuestError;
use thiserror::Error;

/// Failures in the source-actor bridge.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ActorError {
    /// The source frame period is not positive.
    #[error("native owned actors require a positive source frame period")]
    BadFramePeriod,
    /// An actor pointer left the source table.
    #[error("native actor pointer is outside its source table")]
    OutsideTable,
    /// The source refused to release its owned actor.
    #[error("native source refused to release its owned actor")]
    RefusedRelease,
    /// An owned-actor field exceeds the source stride.
    #[error("native owned-actor field exceeds the source stride")]
    FieldRange,
    /// A named export is not declared.
    #[error("native entry export is not declared: {0}")]
    MissingExport(String),
    /// No guest body is registered for an entry.
    #[error("native entry has no guest body: {0:#x}")]
    MissingBody(u64),
    /// A saved actor does not belong to this mod and slot.
    #[error("saved native owned actor does not belong here: {0}")]
    SaveMismatch(String),
    /// A restore lost an owned source actor.
    #[error("original native save lost an owned source actor")]
    LostRestoreActor,
    /// Underlying guest failure.
    #[error("native actor guest failure: {0}")]
    Guest(String),
}

impl From<GuestError> for ActorError {
    fn from(error: GuestError) -> Self {
        Self::Guest(error.to_string())
    }
}

/// Generational actor handle local to the actor bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeActorId {
    /// Registry slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
}

/// Save-safe actor reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SavedActorId {
    /// Registry slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
}

impl From<NativeActorId> for SavedActorId {
    fn from(id: NativeActorId) -> Self {
        Self {
            slot: id.slot,
            generation: id.generation,
        }
    }
}

/// Actor handle with its owning provider name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedActor {
    /// Actor handle.
    pub id: NativeActorId,
    /// Owning provider.
    pub owner: String,
}

/// Native entry reference: export name or image RVA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryRef {
    /// Named export.
    Export(String),
    /// Image-relative offset.
    Rva(u64),
}

/// Scalar field inside a source entity record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalarField {
    /// Byte offset inside the record.
    pub offset: usize,
    /// Lane storage.
    pub storage: GuestStorage,
}

impl ScalarField {
    fn width(&self, pointer_bytes: usize) -> usize {
        self.storage.byte_length(pointer_bytes)
    }
}

/// Owned-actor shared field layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceActorFields {
    /// Velocity vector offset.
    pub velocity: usize,
    /// Ground pointer offset.
    pub ground: usize,
    /// Think pointer offset.
    pub think: usize,
    /// Use pointer offset, if any.
    pub use_offset: Option<usize>,
    /// Next-think scalar.
    pub nextthink: ScalarField,
}

/// Clock input kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockInput {
    /// Source frame counter.
    Frame,
    /// Source time.
    Time,
}

/// Clock units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockUnits {
    /// Seconds.
    Seconds,
    /// Milliseconds.
    Milliseconds,
}

/// One source clock projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockField {
    /// Destination address.
    pub address: EntryRef,
    /// Input kind.
    pub input: ClockInput,
    /// Time units.
    pub units: ClockUnits,
    /// Lane storage.
    pub encoding: GuestStorage,
}

/// Source-actor declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceActorDefinition {
    /// Source frame period in seconds.
    pub frame_seconds: f64,
    /// Clock projections.
    pub clock: Vec<ClockField>,
    /// Shared field layout.
    pub fields: SourceActorFields,
    /// Allocate entry.
    pub allocate: EntryRef,
    /// Release entry.
    pub release: EntryRef,
    /// Per-frame update entry.
    pub update: EntryRef,
}

/// Body state mapped onto source words.
#[derive(Debug, Clone, PartialEq)]
pub struct BodyState {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Bounds minimum.
    pub bounds_min: Vec3,
    /// Bounds maximum.
    pub bounds_max: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Ground actor, if any.
    pub ground: Option<NativeActorId>,
}

/// Saved behavior record passthrough.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedBehaviorHit {
    /// Owning actor.
    pub target: NativeActorId,
    /// Retained amount.
    pub amount: f64,
}

/// Combat/callback behavior owned alongside the actors.
pub trait ActorBehavior {
    /// Bind behavior for one adopted slot.
    fn bind(&mut self, slot: usize, actor: OwnedActor);
    /// Validate behavior state.
    fn validate(&self) -> Result<(), ActorError>;
    /// Release behavior for one actor.
    fn release(&mut self, actor: NativeActorId);
    /// Capture behavior state.
    fn checkpoint(&self) -> Vec<SavedBehaviorHit>;
    /// Validate saved behavior records.
    fn validate_saved(&self, records: &[SavedBehaviorHit]) -> Result<(), ActorError>;
    /// Restore behavior records.
    fn restore(&mut self, records: &[SavedBehaviorHit]) -> Result<(), ActorError>;
    /// Suspend or resume behavior.
    fn suspend(&mut self, value: bool);
    /// Release all behavior state.
    fn close(&mut self);
}

/// Actor registry surface the bridge needs.
pub trait ActorStore {
    /// Allocate an owned actor at a source slot.
    fn allocate_at_source(&mut self, slot: usize, label: &str) -> OwnedActor;
    /// Whether an actor is live.
    fn is_live(&self, actor: NativeActorId) -> bool;
    /// Release an owned actor.
    fn release_owned(&mut self, actor: OwnedActor);
    /// Resolve a saved reference to a live actor.
    fn reference_saved(&mut self, saved: SavedActorId) -> NativeActorId;
    /// Resolve a live owned actor.
    fn resolve_owned(&self, actor: NativeActorId) -> Option<OwnedActor>;
    /// Source slot of an actor, if bound.
    fn source_slot(&self, actor: NativeActorId) -> Option<usize>;
    /// Drain externally released actors.
    fn take_released(&mut self) -> Vec<NativeActorId>;
}

/// Body binding table indexed by adopted slot.
pub trait BodyTable {
    /// Record the slot binding for an actor.
    fn rebind(&mut self, actor: NativeActorId, slot: usize);
    /// Drop the slot binding for an actor.
    fn unbind(&mut self, actor: NativeActorId);
    /// Whether an actor is linked.
    fn linked(&self, actor: NativeActorId) -> bool;
    /// Link an actor.
    fn link(&mut self, actor: NativeActorId);
}

/// Per-frame source hooks: client rows, clock sync, and frame brackets.
pub trait SourceActorCalls {
    /// Whether a slot runs on the client clock instead.
    fn client_frame(&self, slot: usize) -> bool;
    /// Publish source time and frame to the guest.
    fn synchronize_frame(&self, seconds: f64, frame: u64);
    /// Open a source frame.
    fn begin_frame(&self);
    /// Close a source frame.
    fn end_frame(&self);
    /// Current shared time in seconds.
    fn now(&self) -> f64;
}

/// Guest body invoked by the synthetic actor host.
pub type HostBody = dyn Fn(&mut SyntheticActorHost, &[GuestCallValue]) -> GuestCallResult;

/// Which lifecycle interception an entry carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Intercept {
    Allocate,
    Release,
}

/// Headless source-actor host: entity table, guest memory, and guest entries.
pub struct SyntheticActorHost {
    /// Guest memory.
    pub memory: SparseGuestMemory,
    /// Image base address.
    pub image_base: GuestAddress,
    table_base: GuestAddress,
    stride: usize,
    count: usize,
    active: Vec<bool>,
    exports: HashMap<String, GuestAddress>,
    bodies: HashMap<u64, std::rc::Rc<HostBody>>,
    intercepts: HashMap<u64, Intercept>,
    /// Allocate results awaiting adoption (offsets or null).
    pub allocate_events: Vec<Option<u64>>,
    /// Release calls awaiting retirement (offsets).
    pub release_events: Vec<u64>,
    /// Presentation releases issued.
    pub presentation_releases: Vec<NativeActorId>,
}

impl SyntheticActorHost {
    /// Build a host over a mapped entity table.
    pub fn new(
        memory: SparseGuestMemory,
        image_base: GuestAddress,
        table_base: GuestAddress,
        stride: usize,
        count: usize,
    ) -> Self {
        Self {
            memory,
            image_base,
            table_base,
            stride,
            count,
            active: vec![false; count],
            exports: HashMap::new(),
            bodies: HashMap::new(),
            intercepts: HashMap::new(),
            allocate_events: Vec::new(),
            release_events: Vec::new(),
            presentation_releases: Vec::new(),
        }
    }

    /// Entity table geometry.
    #[must_use]
    pub fn table(&self) -> (GuestAddress, usize, usize) {
        (self.table_base, self.stride, self.count)
    }

    /// Register a named export address.
    pub fn register_export(&mut self, name: &str, address: GuestAddress) {
        self.exports.insert(name.to_string(), address);
    }

    /// Register a guest body at an offset.
    pub fn register_body(
        &mut self,
        offset: u64,
        body: impl Fn(&mut SyntheticActorHost, &[GuestCallValue]) -> GuestCallResult + 'static,
    ) {
        self.bodies.insert(offset, std::rc::Rc::new(body));
    }

    /// Whether a slot is active.
    #[must_use]
    pub fn is_active(&self, slot: usize) -> bool {
        self.active.get(slot).copied().unwrap_or(false)
    }

    /// Set slot activity.
    pub fn set_active(&mut self, slot: usize, active: bool) {
        if let Some(flag) = self.active.get_mut(slot) {
            *flag = active;
        }
    }

    /// Base address of one entity slot.
    pub fn entity_address(&self, slot: usize) -> Result<GuestAddress, ActorError> {
        if slot >= self.count {
            return Err(ActorError::OutsideTable);
        }
        Ok(self.memory.offset(self.table_base, (slot * self.stride) as i64)?)
    }

    /// Table slot for an entity address.
    pub fn slot_of(&self, address: GuestAddress) -> Result<usize, ActorError> {
        if address.offset < self.table_base.offset {
            return Err(ActorError::OutsideTable);
        }
        let relative = address.offset - self.table_base.offset;
        if !relative.is_multiple_of(self.stride as u64) || relative / self.stride as u64 >= self.count as u64 {
            return Err(ActorError::OutsideTable);
        }
        Ok((relative / self.stride as u64) as usize)
    }

    /// Invoke a guest entry, recording lifecycle interceptions.
    pub fn invoke_entry(
        &mut self,
        address: GuestAddress,
        values: &[GuestCallValue],
    ) -> Result<GuestCallResult, ActorError> {
        let body = self
            .bodies
            .get(&address.offset)
            .cloned()
            .ok_or(ActorError::MissingBody(address.offset))?;
        let result = body(self, values);
        match self.intercepts.get(&address.offset) {
            Some(Intercept::Allocate) => match &result {
                GuestCallResult::Value(GuestCallValue::Pointer(Some(address))) => {
                    self.allocate_events.push(Some(address.offset));
                }
                _ => self.allocate_events.push(None),
            },
            Some(Intercept::Release) => {
                if let Some(GuestCallValue::Pointer(Some(address))) = values.first() {
                    self.release_events.push(address.offset);
                }
            }
            None => {}
        }
        Ok(result)
    }

    /// Record a presentation release.
    pub fn presentation_release(&mut self, actor: NativeActorId) {
        self.presentation_releases.push(actor);
    }

    /// Presentation releases issued so far.
    #[must_use]
    pub fn releases(&self) -> &[NativeActorId] {
        &self.presentation_releases
    }
}

fn f64_to_scalar(value: f64, storage: GuestStorage, pointer_bytes: usize) -> Vec<u8> {
    match storage {
        GuestStorage::Float32 => (value as f32).to_le_bytes().to_vec(),
        GuestStorage::Float64 => value.to_le_bytes().to_vec(),
        GuestStorage::Int8 | GuestStorage::Uint8 => vec![value as i64 as u8],
        GuestStorage::Int16 | GuestStorage::Uint16 => (value as i64 as i16).to_le_bytes().to_vec(),
        GuestStorage::Int32 | GuestStorage::Uint32 => (value as i64 as i32).to_le_bytes().to_vec(),
        GuestStorage::Int64 | GuestStorage::Uint64 => (value as i64).to_le_bytes().to_vec(),
        GuestStorage::Pointer => {
            if pointer_bytes == 4 {
                (value as u64 as u32).to_le_bytes().to_vec()
            } else {
                (value as u64).to_le_bytes().to_vec()
            }
        }
    }
}

fn vec_to_bytes(value: Vec3) -> [u8; 12] {
    let mut bytes = [0u8; 12];
    bytes[..4].copy_from_slice(&value.x.to_le_bytes());
    bytes[4..8].copy_from_slice(&value.y.to_le_bytes());
    bytes[8..].copy_from_slice(&value.z.to_le_bytes());
    bytes
}

/// Saved owned-actor entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedActorEntry {
    /// Saved actor.
    pub actor: SavedActorId,
    /// Source slot.
    pub slot: usize,
    /// Whether the body was linked.
    pub linked: bool,
}

/// Saved source-actor state.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedNativeActors {
    /// Next frame time.
    pub next_frame: f64,
    /// Source frame.
    pub frame: u64,
    /// Saved behavior records.
    pub deferred: Vec<SavedBehaviorHit>,
    /// Saved actors.
    pub actors: Vec<SavedActorEntry>,
}

/// Validated restore entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedActor {
    /// Owned actor.
    pub actor: OwnedActor,
    /// Source slot.
    pub slot: usize,
    /// Whether the body was linked.
    pub linked: bool,
}

/// Source-owned actor bridge over a synthetic host.
pub struct NativeModActors<C: SourceActorCalls, B: ActorBehavior> {
    definition: SourceActorDefinition,
    entity_record: Option<(usize, usize)>,
    instance: String,
    calls: C,
    behavior: B,
    slots: HashMap<usize, OwnedActor>,
    actor_slots: HashMap<NativeActorId, usize>,
    pending: HashSet<usize>,
    suspended: bool,
    closing: bool,
    frame: u64,
    next_frame: f64,
    tick_time: Option<f64>,
}

impl<C: SourceActorCalls, B: ActorBehavior> NativeModActors<C, B> {
    /// Build the bridge, arming allocate/release interceptions.
    pub fn new(
        definition: SourceActorDefinition,
        entity_record: Option<(usize, usize)>,
        instance: &str,
        host: &mut SyntheticActorHost,
        calls: C,
        behavior: B,
    ) -> Result<Self, ActorError> {
        if !definition.frame_seconds.is_finite() || definition.frame_seconds <= 0.0 {
            return Err(ActorError::BadFramePeriod);
        }
        let next_frame = calls.now() + definition.frame_seconds;
        let allocate = Self::entry(host, &definition.allocate)?;
        let release = Self::entry(host, &definition.release)?;
        host.intercepts.insert(allocate.offset, Intercept::Allocate);
        host.intercepts.insert(release.offset, Intercept::Release);
        Ok(Self {
            definition,
            entity_record,
            instance: instance.to_string(),
            calls,
            behavior,
            slots: HashMap::new(),
            actor_slots: HashMap::new(),
            pending: HashSet::new(),
            suspended: false,
            closing: false,
            frame: 0,
            next_frame,
            tick_time: None,
        })
    }

    /// Whether a source tick is currently executing.
    #[must_use]
    pub fn advancing(&self) -> bool {
        self.tick_time.is_some()
    }

    /// Current source frame.
    #[must_use]
    pub fn source_frame(&self) -> u64 {
        self.frame
    }

    /// Current source time.
    #[must_use]
    pub fn source_time(&self) -> f64 {
        self.tick_time.unwrap_or_else(|| self.calls.now())
    }

    fn entry(host: &SyntheticActorHost, entry: &EntryRef) -> Result<GuestAddress, ActorError> {
        match entry {
            EntryRef::Export(name) => host
                .exports
                .get(name)
                .copied()
                .ok_or_else(|| ActorError::MissingExport(name.clone())),
            EntryRef::Rva(rva) => Ok(host
                .memory
                .offset(host.image_base, i64::try_from(*rva).unwrap_or(i64::MAX))?),
        }
    }

    fn reserved(&self, slot: usize) -> bool {
        if slot == 0 {
            return true;
        }
        self.entity_record
            .is_some_and(|(first, capacity)| slot >= first && slot < first + capacity)
    }

    fn at(host: &SyntheticActorHost, slot: usize, offset: usize) -> Result<GuestAddress, ActorError> {
        let base = host.entity_address(slot)?;
        Ok(host.memory.offset(base, offset as i64)?)
    }

    /// Live owned actor at a slot, if any.
    pub fn actor_at(&self, store: &impl ActorStore, slot: usize) -> Option<OwnedActor> {
        self.slots.get(&slot).filter(|actor| store.is_live(actor.id)).cloned()
    }

    /// Source slot of an actor, if adopted.
    #[must_use]
    pub fn slot_of(&self, actor: NativeActorId) -> Option<usize> {
        self.actor_slots.get(&actor).copied()
    }

    /// Live adopted entries.
    pub fn entries(&self, store: &impl ActorStore) -> Vec<(NativeActorId, usize)> {
        self.slots
            .iter()
            .filter(|(_, actor)| store.is_live(actor.id))
            .map(|(slot, actor)| (actor.id, *slot))
            .collect()
    }

    fn adopt(
        &mut self,
        host: &mut SyntheticActorHost,
        store: &mut impl ActorStore,
        bodies: &mut impl BodyTable,
        slot: usize,
    ) -> Result<Option<OwnedActor>, ActorError> {
        if self.reserved(slot) || !host.is_active(slot) {
            return Ok(None);
        }
        if let Some(prior) = self.slots.get(&slot) {
            return Ok(Some(prior.clone()));
        }
        let actor = store.allocate_at_source(slot, "native:mod-actor");
        self.slots.insert(slot, actor.clone());
        self.actor_slots.insert(actor.id, slot);
        if let Err(error) = self.bind(bodies, slot, actor.clone()) {
            self.retire(host, store, bodies, slot);
            return Err(error);
        }
        Ok(Some(actor))
    }

    fn bind(&mut self, bodies: &mut impl BodyTable, slot: usize, actor: OwnedActor) -> Result<(), ActorError> {
        bodies.rebind(actor.id, slot);
        self.behavior.bind(slot, actor);
        Ok(())
    }

    /// Read body state for an adopted slot from source words.
    pub fn read_body(
        &self,
        host: &mut SyntheticActorHost,
        slot: usize,
        resolve: &dyn Fn(usize) -> Option<NativeActorId>,
    ) -> Result<BodyState, ActorError> {
        let fields = &self.definition.fields;
        let origin = host.memory.read_f32x3(Self::at(host, slot, 4)?)?;
        let angles = host.memory.read_f32x3(Self::at(host, slot, 16)?)?;
        let bounds_min = host.memory.read_f32x3(Self::at(host, slot, 188)?)?;
        let bounds_max = host.memory.read_f32x3(Self::at(host, slot, 200)?)?;
        let velocity = host.memory.read_f32x3(Self::at(host, slot, fields.velocity)?)?;
        let ground = host.memory.read_pointer(Self::at(host, slot, fields.ground)?)?;
        let ground = ground
            .map(|address| host.slot_of(address))
            .transpose()?
            .and_then(resolve);
        Ok(BodyState {
            origin,
            angles,
            bounds_min,
            bounds_max,
            velocity,
            ground,
        })
    }

    /// Write body state for an adopted slot into source words.
    pub fn write_body(
        &self,
        host: &mut SyntheticActorHost,
        slot: usize,
        address_of: &dyn Fn(NativeActorId) -> Option<GuestAddress>,
        state: &BodyState,
    ) -> Result<(), ActorError> {
        let fields = &self.definition.fields;
        host.memory
            .write(Self::at(host, slot, 4)?, &vec_to_bytes(state.origin))?;
        host.memory
            .write(Self::at(host, slot, 16)?, &vec_to_bytes(state.angles))?;
        host.memory
            .write(Self::at(host, slot, 188)?, &vec_to_bytes(state.bounds_min))?;
        host.memory
            .write(Self::at(host, slot, 200)?, &vec_to_bytes(state.bounds_max))?;
        host.memory
            .write(Self::at(host, slot, fields.velocity)?, &vec_to_bytes(state.velocity))?;
        let ground = state.ground.and_then(address_of);
        host.memory
            .write_pointer(Self::at(host, slot, fields.ground)?, ground)?;
        Ok(())
    }

    /// Publish source time and frame into guest clock words.
    pub fn synchronize_clock(&self, host: &mut SyntheticActorHost) -> Result<(), ActorError> {
        if self.suspended {
            return Ok(());
        }
        let time = self.source_time();
        self.calls.synchronize_frame(time, self.frame);
        for field in &self.definition.clock {
            let value = match field.input {
                ClockInput::Frame => self.frame as f64,
                ClockInput::Time => {
                    time * f64::from(match field.units {
                        ClockUnits::Milliseconds => 1000,
                        ClockUnits::Seconds => 1,
                    })
                }
            };
            let value = match (&field.input, &field.units, &field.encoding) {
                (ClockInput::Time, ClockUnits::Milliseconds, GuestStorage::Float32)
                | (ClockInput::Time, ClockUnits::Milliseconds, GuestStorage::Float64) => value,
                (ClockInput::Time, ClockUnits::Milliseconds, _) => value.round(),
                _ => value,
            };
            let address = Self::entry(host, &field.address)?;
            let bytes = f64_to_scalar(value, field.encoding, host.memory.pointer_bytes());
            host.memory.write(address, &bytes)?;
        }
        Ok(())
    }

    /// Validate field bounds, the update entry, and behavior state.
    pub fn validate(&mut self, host: &mut SyntheticActorHost) -> Result<(), ActorError> {
        let (_, stride, _) = host.table();
        let pointer_bytes = host.memory.pointer_bytes();
        let fields = &self.definition.fields;
        let mut checks = vec![
            (fields.velocity, 12),
            (fields.ground, pointer_bytes),
            (fields.think, pointer_bytes),
            (fields.nextthink.offset, fields.nextthink.width(pointer_bytes)),
        ];
        if let Some(use_offset) = fields.use_offset {
            checks.push((use_offset, pointer_bytes));
        }
        for (offset, width) in checks {
            if offset + width > stride {
                return Err(ActorError::FieldRange);
            }
        }
        let update = Self::entry(host, &self.definition.update)?;
        host.memory.check(update, 1, GuestAccess::Execute)?;
        self.behavior.validate()?;
        self.synchronize_clock(host)
    }

    /// Drain externally released actors into the pending set.
    pub fn note_released(&mut self, host: &mut SyntheticActorHost, store: &mut impl ActorStore) {
        for actor in store.take_released() {
            if let Some(slot) = self.actor_slots.get(&actor) {
                self.pending.insert(*slot);
                host.presentation_release(actor);
            }
        }
    }

    /// Process allocate/release interceptions recorded by the host.
    pub fn drain_intercepts(
        &mut self,
        host: &mut SyntheticActorHost,
        store: &mut impl ActorStore,
        bodies: &mut impl BodyTable,
    ) -> Result<(), ActorError> {
        for event in std::mem::take(&mut host.allocate_events) {
            if self.suspended {
                continue;
            }
            let Some(offset) = event else { continue };
            let address = GuestAddress::new(host.memory.address_space(), offset);
            let slot = host.slot_of(address)?;
            self.retire(host, store, bodies, slot);
            self.adopt(host, store, bodies, slot)?;
        }
        for offset in std::mem::take(&mut host.release_events) {
            if self.suspended {
                continue;
            }
            let address = GuestAddress::new(host.memory.address_space(), offset);
            let Ok(slot) = host.slot_of(address) else { continue };
            if !host.is_active(slot) {
                self.retire(host, store, bodies, slot);
            }
        }
        Ok(())
    }

    /// Release pending slots through the guest release entry.
    pub fn drain_releases(
        &mut self,
        host: &mut SyntheticActorHost,
        store: &mut impl ActorStore,
        bodies: &mut impl BodyTable,
    ) -> Result<(), ActorError> {
        if self.suspended {
            return Ok(());
        }
        self.note_released(host, store);
        let release = Self::entry(host, &self.definition.release)?;
        for slot in std::mem::take(&mut self.pending) {
            if host.is_active(slot) {
                let address = host.entity_address(slot)?;
                host.invoke_entry(release, &[GuestCallValue::Pointer(Some(address))])?;
                host.release_events.clear();
            }
            if !self.closing && host.is_active(slot) {
                self.pending.insert(slot);
                return Err(ActorError::RefusedRelease);
            }
            self.retire(host, store, bodies, slot);
        }
        Ok(())
    }

    fn retire(
        &mut self,
        host: &mut SyntheticActorHost,
        store: &mut impl ActorStore,
        bodies: &mut impl BodyTable,
        slot: usize,
    ) {
        let actor = self.slots.remove(&slot);
        self.pending.remove(&slot);
        let Some(actor) = actor else { return };
        self.behavior.release(actor.id);
        self.actor_slots.remove(&actor.id);
        bodies.unbind(actor.id);
        host.presentation_release(actor.id);
        if store.is_live(actor.id) {
            store.release_owned(actor);
        }
    }

    /// Advance the source clock, running update for owned slots.
    pub fn advance(
        &mut self,
        host: &mut SyntheticActorHost,
        store: &mut impl ActorStore,
        bodies: &mut impl BodyTable,
        now: f64,
    ) -> Result<(), ActorError> {
        if self.suspended || self.closing {
            return Ok(());
        }
        self.drain_releases(host, store, bodies)?;
        while !self.closing && self.next_frame <= now + 1e-9 {
            self.tick_time = Some(self.next_frame);
            self.next_frame += self.definition.frame_seconds;
            self.frame += 1;
            let tick = self.tick(host, store, bodies);
            self.tick_time = None;
            tick?;
        }
        Ok(())
    }

    fn tick(
        &mut self,
        host: &mut SyntheticActorHost,
        store: &mut impl ActorStore,
        bodies: &mut impl BodyTable,
    ) -> Result<(), ActorError> {
        self.synchronize_clock(host)?;
        self.calls.begin_frame();
        let update = Self::entry(host, &self.definition.update)?;
        let (_, _, count) = host.table();
        for slot in 0..count {
            if self.closing || self.calls.client_frame(slot) {
                continue;
            }
            let Some(actor) = self.slots.get(&slot).cloned() else {
                continue;
            };
            if !store.is_live(actor.id) || !host.is_active(slot) {
                self.retire(host, store, bodies, slot);
                continue;
            }
            let address = host.entity_address(slot)?;
            host.invoke_entry(update, &[GuestCallValue::Pointer(Some(address))])?;
        }
        if !self.closing {
            self.calls.end_frame();
        }
        Ok(())
    }

    /// Suspend or resume the source clock.
    pub fn suspend(&mut self, value: bool) {
        self.suspended = value;
        self.behavior.suspend(value);
    }

    /// Capture owned-actor state.
    pub fn checkpoint(&self, store: &impl ActorStore, bodies: &impl BodyTable) -> SavedNativeActors {
        SavedNativeActors {
            next_frame: self.next_frame,
            frame: self.frame,
            deferred: self.behavior.checkpoint(),
            actors: self
                .entries(store)
                .iter()
                .map(|(actor, slot)| SavedActorEntry {
                    actor: SavedActorId::from(*actor),
                    slot: *slot,
                    linked: bodies.linked(*actor),
                })
                .collect(),
        }
    }

    /// Validate saved actors against live ownership.
    pub fn validate_saved(
        &self,
        store: &mut impl ActorStore,
        saved: &SavedNativeActors,
    ) -> Result<Vec<ValidatedActor>, ActorError> {
        self.behavior.validate_saved(&saved.deferred)?;
        saved
            .actors
            .iter()
            .map(|entry| {
                let id = store.reference_saved(entry.actor);
                let actor = store.resolve_owned(id).ok_or_else(|| {
                    ActorError::SaveMismatch(format!("{}/{}", entry.actor.slot, entry.actor.generation))
                })?;
                if actor.owner != self.instance
                    || self.reserved(entry.slot)
                    || store.source_slot(id) != Some(entry.slot)
                {
                    return Err(ActorError::SaveMismatch(format!(
                        "{}/{}",
                        entry.actor.slot, entry.actor.generation
                    )));
                }
                Ok(ValidatedActor {
                    actor,
                    slot: entry.slot,
                    linked: entry.linked,
                })
            })
            .collect()
    }

    /// Restore owned actors from validated entries.
    pub fn restore(
        &mut self,
        host: &mut SyntheticActorHost,
        _store: &mut impl ActorStore,
        bodies: &mut impl BodyTable,
        saved: &SavedNativeActors,
        actors: &[ValidatedActor],
    ) -> Result<(), ActorError> {
        let (_, _, count) = host.table();
        for entry in actors {
            if entry.slot >= count || !host.is_active(entry.slot) {
                return Err(ActorError::LostRestoreActor);
            }
        }
        self.slots.clear();
        self.actor_slots.clear();
        self.pending.clear();
        self.frame = saved.frame;
        self.next_frame = saved.next_frame;
        for entry in actors {
            self.slots.insert(entry.slot, entry.actor.clone());
            self.actor_slots.insert(entry.actor.id, entry.slot);
        }
        for entry in actors {
            self.bind(bodies, entry.slot, entry.actor.clone())?;
            if entry.linked {
                bodies.link(entry.actor.id);
            }
        }
        self.behavior.restore(&saved.deferred)
    }

    /// Release every owned actor and behavior state.
    pub fn close(
        &mut self,
        host: &mut SyntheticActorHost,
        store: &mut impl ActorStore,
        bodies: &mut impl BodyTable,
    ) -> Result<(), ActorError> {
        if self.closing {
            return Ok(());
        }
        self.closing = true;
        for slot in self.slots.keys().copied().collect::<Vec<_>>() {
            self.pending.insert(slot);
        }
        let drained = self.drain_releases(host, store, bodies);
        for slot in self.slots.keys().copied().collect::<Vec<_>>() {
            self.retire(host, store, bodies, slot);
        }
        self.behavior.close();
        host.intercepts.clear();
        drained
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, GuestMapOptions, GuestPermissions, ModuleIdentity};
    use std::cell::RefCell;
    use std::rc::Rc;

    const STRIDE: usize = 256;
    const ALLOC: u64 = 0x7000;
    const RELEASE: u64 = 0x7100;
    const UPDATE: u64 = 0x7200;

    fn actor(slot: u32) -> NativeActorId {
        NativeActorId { slot, generation: 1 }
    }

    fn definition() -> SourceActorDefinition {
        SourceActorDefinition {
            frame_seconds: 0.1,
            clock: vec![ClockField {
                address: EntryRef::Rva(0x100),
                input: ClockInput::Time,
                units: ClockUnits::Seconds,
                encoding: GuestStorage::Float64,
            }],
            fields: SourceActorFields {
                velocity: 64,
                ground: 80,
                think: 88,
                use_offset: Some(96),
                nextthink: ScalarField {
                    offset: 100,
                    storage: GuestStorage::Int32,
                },
            },
            allocate: EntryRef::Rva(ALLOC),
            release: EntryRef::Rva(RELEASE),
            update: EntryRef::Rva(UPDATE),
        }
    }

    struct TestStore {
        live: HashSet<NativeActorId>,
        owners: HashMap<NativeActorId, OwnedActor>,
        slots: HashMap<NativeActorId, usize>,
        released: Vec<NativeActorId>,
    }

    impl ActorStore for TestStore {
        fn allocate_at_source(&mut self, slot: usize, label: &str) -> OwnedActor {
            assert_eq!(label, "native:mod-actor");
            let id = actor(slot as u32);
            let owned = OwnedActor {
                id,
                owner: "test:mod".to_string(),
            };
            self.live.insert(id);
            self.owners.insert(id, owned.clone());
            self.slots.insert(id, slot);
            owned
        }

        fn is_live(&self, actor: NativeActorId) -> bool {
            self.live.contains(&actor)
        }

        fn release_owned(&mut self, actor: OwnedActor) {
            self.live.remove(&actor.id);
        }

        fn reference_saved(&mut self, saved: SavedActorId) -> NativeActorId {
            NativeActorId {
                slot: saved.slot,
                generation: saved.generation,
            }
        }

        fn resolve_owned(&self, actor: NativeActorId) -> Option<OwnedActor> {
            self.owners.get(&actor).cloned()
        }

        fn source_slot(&self, actor: NativeActorId) -> Option<usize> {
            self.slots.get(&actor).copied()
        }

        fn take_released(&mut self) -> Vec<NativeActorId> {
            std::mem::take(&mut self.released)
        }
    }

    struct TestBodies {
        slots: HashMap<NativeActorId, usize>,
        linked: HashSet<NativeActorId>,
    }

    impl BodyTable for TestBodies {
        fn rebind(&mut self, actor: NativeActorId, slot: usize) {
            self.slots.insert(actor, slot);
        }

        fn unbind(&mut self, actor: NativeActorId) {
            self.slots.remove(&actor);
        }

        fn linked(&self, actor: NativeActorId) -> bool {
            self.linked.contains(&actor)
        }

        fn link(&mut self, actor: NativeActorId) {
            self.linked.insert(actor);
        }
    }

    struct TestBehavior {
        bound: Vec<(usize, NativeActorId)>,
        hits: Vec<SavedBehaviorHit>,
    }

    impl ActorBehavior for TestBehavior {
        fn bind(&mut self, slot: usize, actor: OwnedActor) {
            self.bound.push((slot, actor.id));
        }

        fn validate(&self) -> Result<(), ActorError> {
            Ok(())
        }

        fn release(&mut self, actor: NativeActorId) {
            self.bound.retain(|(_, bound)| *bound != actor);
        }

        fn checkpoint(&self) -> Vec<SavedBehaviorHit> {
            self.hits.clone()
        }

        fn validate_saved(&self, _records: &[SavedBehaviorHit]) -> Result<(), ActorError> {
            Ok(())
        }

        fn restore(&mut self, records: &[SavedBehaviorHit]) -> Result<(), ActorError> {
            self.hits = records.to_vec();
            Ok(())
        }

        fn suspend(&mut self, _value: bool) {}

        fn close(&mut self) {}
    }

    struct TestCalls {
        now: Rc<RefCell<f64>>,
        client_slots: HashSet<usize>,
        frames: Rc<RefCell<Vec<(f64, u64)>>>,
        begins: Rc<RefCell<u32>>,
        ends: Rc<RefCell<u32>>,
    }

    impl SourceActorCalls for TestCalls {
        fn client_frame(&self, slot: usize) -> bool {
            self.client_slots.contains(&slot)
        }

        fn synchronize_frame(&self, seconds: f64, frame: u64) {
            self.frames.borrow_mut().push((seconds, frame));
        }

        fn begin_frame(&self) {
            *self.begins.borrow_mut() += 1;
        }

        fn end_frame(&self) {
            *self.ends.borrow_mut() += 1;
        }

        fn now(&self) -> f64 {
            *self.now.borrow()
        }
    }

    struct Fixture {
        actors: NativeModActors<TestCalls, TestBehavior>,
        host: SyntheticActorHost,
        store: TestStore,
        bodies: TestBodies,
        now: Rc<RefCell<f64>>,
        updates: Rc<RefCell<Vec<u64>>>,
    }

    fn fixture() -> Fixture {
        let module = ModuleIdentity::new(
            ProviderId::new("test", "actors"),
            "actors.so",
            ContentDigest {
                algorithm: "none".to_string(),
                value: "0".to_string(),
            },
            "r1",
        );
        let mut memory = SparseGuestMemory::new(module, 4, 0xB0000).unwrap();
        let image_base = memory
            .map(&GuestMapOptions::new(
                0x10000,
                0x8000,
                GuestPermissions::ReadWriteExecute,
            ))
            .unwrap();
        let table_base = memory
            .map(&GuestMapOptions::new(0x20000, STRIDE * 8, GuestPermissions::ReadWrite))
            .unwrap();
        let mut host = SyntheticActorHost::new(memory, image_base, table_base, STRIDE, 8);
        let code = image_base.offset;
        host.register_body(code + ALLOC, |host, _| {
            for slot in 1..8 {
                if !host.is_active(slot) {
                    host.set_active(slot, true);
                    let address = host.entity_address(slot).unwrap();
                    return GuestCallResult::Value(GuestCallValue::Pointer(Some(address)));
                }
            }
            GuestCallResult::Value(GuestCallValue::Pointer(None))
        });
        host.register_body(code + RELEASE, |host, values| {
            if let Some(GuestCallValue::Pointer(Some(address))) = values.first() {
                if let Ok(slot) = host.slot_of(*address) {
                    host.set_active(slot, false);
                }
            }
            GuestCallResult::Void
        });
        let updates = Rc::new(RefCell::new(Vec::new()));
        let seen = updates.clone();
        host.register_body(code + UPDATE, move |host, values| {
            if let Some(GuestCallValue::Pointer(Some(address))) = values.first() {
                seen.borrow_mut().push(host.slot_of(*address).unwrap() as u64);
            }
            GuestCallResult::Void
        });
        let now = Rc::new(RefCell::new(1.0));
        let calls = TestCalls {
            now: now.clone(),
            client_slots: HashSet::new(),
            frames: Rc::new(RefCell::new(Vec::new())),
            begins: Rc::new(RefCell::new(0)),
            ends: Rc::new(RefCell::new(0)),
        };
        let behavior = TestBehavior {
            bound: Vec::new(),
            hits: Vec::new(),
        };
        let actors = NativeModActors::new(definition(), Some((1, 2)), "test:mod", &mut host, calls, behavior).unwrap();
        Fixture {
            actors,
            host,
            store: TestStore {
                live: HashSet::new(),
                owners: HashMap::new(),
                slots: HashMap::new(),
                released: Vec::new(),
            },
            bodies: TestBodies {
                slots: HashMap::new(),
                linked: HashSet::new(),
            },
            now,
            updates,
        }
    }

    #[test]
    fn allocate_adopts_and_advance_ticks_update() {
        let mut fixture = fixture();
        fixture.actors.validate(&mut fixture.host).unwrap();
        let alloc = NativeModActors::<TestCalls, TestBehavior>::entry(&fixture.host, &EntryRef::Rva(ALLOC)).unwrap();
        // Slot 1 is reserved (entity record rows); allocate twice for slot 2.
        fixture.host.invoke_entry(alloc, &[]).unwrap();
        fixture
            .actors
            .drain_intercepts(&mut fixture.host, &mut fixture.store, &mut fixture.bodies)
            .unwrap();
        assert!(fixture.actors.entries(&fixture.store).is_empty());
        fixture.host.invoke_entry(alloc, &[]).unwrap();
        fixture.host.invoke_entry(alloc, &[]).unwrap();
        fixture
            .actors
            .drain_intercepts(&mut fixture.host, &mut fixture.store, &mut fixture.bodies)
            .unwrap();
        let entries = fixture.actors.entries(&fixture.store);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].1, 3);

        fixture
            .host
            .memory
            .write(
                SyntheticActorHost::entity_address(&fixture.host, 3).unwrap(),
                &vec![0u8; STRIDE],
            )
            .ok();
        let state = BodyState {
            origin: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            angles: Vec3 {
                x: 0.0,
                y: 90.0,
                z: 0.0,
            },
            bounds_min: Vec3 {
                x: -16.0,
                y: -16.0,
                z: -24.0,
            },
            bounds_max: Vec3 {
                x: 16.0,
                y: 16.0,
                z: 32.0,
            },
            velocity: Vec3 { x: 5.0, y: 0.0, z: 0.0 },
            ground: None,
        };
        fixture
            .actors
            .write_body(&mut fixture.host, 3, &|_| None, &state)
            .unwrap();
        let roundtrip = fixture.actors.read_body(&mut fixture.host, 3, &|_| None).unwrap();
        assert_eq!(roundtrip, state);

        *fixture.now.borrow_mut() = 1.25;
        fixture
            .actors
            .advance(&mut fixture.host, &mut fixture.store, &mut fixture.bodies, 1.25)
            .unwrap();
        assert_eq!(fixture.actors.source_frame(), 2);
        assert_eq!(*fixture.updates.borrow(), vec![3, 3]);
    }

    #[test]
    fn release_drains_and_checkpoint_roundtrips() {
        let mut fixture = fixture();
        let alloc = NativeModActors::<TestCalls, TestBehavior>::entry(&fixture.host, &EntryRef::Rva(ALLOC)).unwrap();
        for _ in 0..4 {
            fixture.host.invoke_entry(alloc, &[]).unwrap();
        }
        fixture
            .actors
            .drain_intercepts(&mut fixture.host, &mut fixture.store, &mut fixture.bodies)
            .unwrap();
        assert_eq!(fixture.actors.entries(&fixture.store).len(), 2);
        fixture.bodies.link(actor(3));
        let saved = fixture.actors.checkpoint(&fixture.store, &fixture.bodies);
        assert_eq!(saved.actors.len(), 2);

        fixture.store.released.push(actor(3));
        fixture
            .actors
            .drain_releases(&mut fixture.host, &mut fixture.store, &mut fixture.bodies)
            .unwrap();
        assert!(!fixture.host.is_active(3));
        assert_eq!(fixture.actors.entries(&fixture.store).len(), 1);

        fixture.host.set_active(3, true);
        fixture.store.live.insert(actor(3));
        let validated = fixture.actors.validate_saved(&mut fixture.store, &saved).unwrap();
        fixture
            .actors
            .restore(
                &mut fixture.host,
                &mut fixture.store,
                &mut fixture.bodies,
                &saved,
                &validated,
            )
            .unwrap();
        assert_eq!(fixture.actors.entries(&fixture.store).len(), 2);
        assert!(fixture.bodies.linked(actor(3)));
        fixture
            .actors
            .close(&mut fixture.host, &mut fixture.store, &mut fixture.bodies)
            .unwrap();
        assert!(fixture.actors.entries(&fixture.store).is_empty());
    }

    #[test]
    fn bad_fields_and_refused_release_fail() {
        let mut fixture = fixture();
        let mut bad = definition();
        bad.fields.velocity = STRIDE;
        let calls = TestCalls {
            now: fixture.now.clone(),
            client_slots: HashSet::new(),
            frames: Rc::new(RefCell::new(Vec::new())),
            begins: Rc::new(RefCell::new(0)),
            ends: Rc::new(RefCell::new(0)),
        };
        let behavior = TestBehavior {
            bound: Vec::new(),
            hits: Vec::new(),
        };
        let mut actors = NativeModActors::new(bad, None, "test:mod", &mut fixture.host, calls, behavior).unwrap();
        assert_eq!(actors.validate(&mut fixture.host), Err(ActorError::FieldRange));

        let alloc = NativeModActors::<TestCalls, TestBehavior>::entry(&fixture.host, &EntryRef::Rva(ALLOC)).unwrap();
        for _ in 0..3 {
            fixture.host.invoke_entry(alloc, &[]).unwrap();
        }
        fixture
            .actors
            .drain_intercepts(&mut fixture.host, &mut fixture.store, &mut fixture.bodies)
            .unwrap();
        // Stubborn release body keeps the slot active.
        let release = fixture.host.image_base.offset + RELEASE;
        fixture.host.register_body(release, |_, _| GuestCallResult::Void);
        fixture.store.released.push(actor(3));
        let refused = fixture
            .actors
            .drain_releases(&mut fixture.host, &mut fixture.store, &mut fixture.bodies);
        assert_eq!(refused, Err(ActorError::RefusedRelease));
    }
}
