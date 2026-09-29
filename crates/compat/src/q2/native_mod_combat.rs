//! Port of `src/compat/q2/native-mod-combat.ts`.
//! Bridges source damage and actor callbacks: the original damage and callback
//! bodies stay guest-owned while eligibility, marshaling, and observation
//! compose on the shared boundary.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_core::math::Vec3;
use qa_guest::core::contracts::{GuestAddress, GuestCallResult, GuestCallValue, GuestLayout, GuestStorage};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::error::GuestError;
use thiserror::Error;

/// Failures in the native combat bridge.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum CombatError {
    /// A callback actor pointer left the source table.
    #[error("native callback actor is outside its source table")]
    ActorOutsideTable,
    /// A callback argument is not an actor pointer.
    #[error("native callback requires an actor pointer")]
    BadActorArgument,
    /// One entry serves two callback kinds.
    #[error("one native callback entry has incompatible declared ABIs")]
    EntryConflict,
    /// A callback field exceeds its source actor.
    #[error("native callback field exceeds its source actor")]
    CallbackFieldRange,
    /// A combat field exceeds its source actor.
    #[error("native combat field exceeds its source actor")]
    CombatFieldRange,
    /// A deferred field exceeds its source actor.
    #[error("native deferred damage field exceeds its source actor")]
    DeferredFieldRange,
    /// Damage ABI and cause edition disagree.
    #[error("native combat ABI differs from its declared damage ABI")]
    AbiMismatch,
    /// Classic damage needs an integer cause.
    #[error("classic damage requires integer MOD")]
    BadClassicCause,
    /// Rerelease damage needs its three-byte aggregate.
    #[error("rerelease damage requires its declared mod_t aggregate")]
    BadRereleaseCause,
    /// Classic touch needs plane and surface pointers.
    #[error("classic touch requires plane and surface pointers")]
    BadTouchArgs,
    /// Rerelease touch needs the shared source trace.
    #[error("native rerelease touch requires the shared source trace")]
    MissingTrace,
    /// A reaction has no native representation.
    #[error("reaction has no native mod_t representation")]
    MissingMod,
    /// A callback target change needs replacement, not redirection.
    #[error("native source callback target changes require replacement")]
    Retargeted,
    /// Shared actor operations are unavailable.
    #[error("native callbacks require shared actor operations")]
    MissingCallbacks,
    /// No damage body was declared.
    #[error("native damage body has not been declared")]
    MissingDamage,
    /// Deferred provenance is missing.
    #[error("native deferred damage has no original attack provenance")]
    MissingProvenance,
    /// Saved deferred target is not owned here.
    #[error("saved deferred damage target is not owned by this native mod")]
    ForeignDeferred,
    /// Underlying guest failure.
    #[error("native combat guest failure: {0}")]
    Guest(String),
}

impl From<GuestError> for CombatError {
    fn from(error: GuestError) -> Self {
        Self::Guest(error.to_string())
    }
}

/// Generational actor handle local to the combat bridge.
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

/// Actor callback kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallbackKind {
    /// Use callback.
    Use,
    /// Touch callback.
    Touch,
    /// Pain callback.
    Pain,
    /// Die callback.
    Die,
}

/// Damage calling convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageAbi {
    /// Classic integer MOD.
    Classic,
    /// Rerelease mod_t aggregate.
    Rerelease,
}

/// Native damage cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeCause {
    /// Classic integer cause.
    Classic(i32),
    /// Rerelease cause record.
    Rerelease {
        /// Cause id.
        id: u8,
        /// Friendly fire flag.
        friendly_fire: bool,
        /// No point-loss flag.
        no_point_loss: bool,
    },
}

/// Damage delivery mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Direct hit.
    Direct,
    /// Radius damage.
    Radius,
}

/// Damage reaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reaction {
    /// No reaction.
    None,
    /// Pain reaction.
    Pain,
    /// Death reaction.
    Death,
}

/// Canonical damage request.
#[derive(Debug, Clone, PartialEq)]
pub struct DamageRequest {
    /// Damage target.
    pub target: NativeActorId,
    /// Attacker, if any.
    pub attacker: Option<NativeActorId>,
    /// Inflictor, if any.
    pub inflictor: Option<NativeActorId>,
    /// Base damage.
    pub amount: f64,
    /// Knockback impulse.
    pub knockback: f64,
    /// Shot direction.
    pub direction: Vec3,
    /// Impact point.
    pub point: Vec3,
    /// Surface normal.
    pub normal: Vec3,
    /// Delivery mode.
    pub delivery: Delivery,
    /// Native cause.
    pub cause: NativeCause,
    /// Damage flags word.
    pub damage_flags: i32,
}

/// Damage outcome.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DamageOutcome {
    /// Reaction taken.
    pub reaction: Reaction,
    /// Damage applied to health.
    pub applied: f64,
}

/// Stored source state published during damage.
#[derive(Debug, Clone, PartialEq)]
pub enum CombatEvent {
    /// Health words changed.
    Health {
        /// Previous health.
        before: f64,
        /// New health.
        after: f64,
    },
    /// Armor counters changed.
    Armor {
        /// Previous armor.
        before: CombatArmor,
        /// New armor.
        after: CombatArmor,
    },
    /// Source velocity changed.
    Velocity {
        /// Previous velocity.
        before: Vec3,
        /// New velocity.
        after: Vec3,
    },
}

/// Synthetic armor counters (points plus cells).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CombatArmor {
    /// Regular points.
    pub points: f64,
    /// Power cells.
    pub cells: f64,
}

/// Combat body state read from source words.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CombatBodyState {
    /// Health.
    pub health: f64,
    /// Mass.
    pub mass: f64,
    /// Armor counters.
    pub armor: CombatArmor,
    /// Whether damage applies.
    pub can_take_damage: bool,
    /// Invulnerable flag.
    pub invulnerable: bool,
    /// No-knockback flag.
    pub no_knockback: bool,
}

/// Contact plane snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContactPlane {
    /// Plane normal.
    pub normal: Vec3,
    /// Plane distance.
    pub distance: f32,
    /// Plane type code.
    pub plane_type: u8,
    /// Sign bits.
    pub signbits: u8,
}

/// Contact surface snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct ContactSurface {
    /// Surface name.
    pub name: String,
    /// Native flags.
    pub flags: i32,
    /// Native value.
    pub value: i32,
}

/// Rerelease source trace snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceSnapshot {
    /// All solid.
    pub all_solid: bool,
    /// Start solid.
    pub start_solid: bool,
    /// Trace fraction.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Contents mask.
    pub contents: u32,
    /// Contact surface.
    pub surface: Option<ContactSurface>,
    /// Hit actor.
    pub hit_actor: NativeActorId,
    /// Inverted flag.
    pub inverted: bool,
}

/// Touch contact across the boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct TouchContact {
    /// Touched actor.
    pub source: NativeActorId,
    /// Other actor.
    pub other: NativeActorId,
    /// Contact plane.
    pub plane: Option<ContactPlane>,
    /// Contact surface.
    pub surface: Option<ContactSurface>,
    /// Rerelease source trace.
    pub trace: Option<TraceSnapshot>,
}

/// Pain reaction across the boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct PainReaction {
    /// Reacting actor.
    pub source: NativeActorId,
    /// Attacker, if any.
    pub attacker: Option<NativeActorId>,
    /// Native cause, if declared.
    pub cause: Option<NativeCause>,
    /// Kick impulse.
    pub kick: f32,
    /// Damage amount.
    pub damage: f64,
}

/// Death reaction across the boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct DeathReaction {
    /// Dying actor.
    pub source: NativeActorId,
    /// Inflictor, if any.
    pub inflictor: Option<NativeActorId>,
    /// Attacker, if any.
    pub attacker: Option<NativeActorId>,
    /// Native cause, if declared.
    pub cause: Option<NativeCause>,
    /// Damage amount.
    pub damage: f64,
    /// Death point.
    pub point: Vec3,
}

/// Declared combat layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatDefinition {
    /// Health field.
    pub health: ScalarField,
    /// Mass field.
    pub mass: ScalarField,
    /// Take-damage flag field.
    pub takedamage: ScalarField,
    /// Flags field.
    pub flags: ScalarField,
    /// Invulnerable bit.
    pub invulnerable_mask: i64,
    /// No-knockback bit.
    pub no_knockback_mask: i64,
    /// Damage entry offset, if declared.
    pub damage_entry: Option<u64>,
    /// Damage ABI.
    pub damage_abi: DamageAbi,
    /// Whether causes use the classic edition.
    pub causes_classic: bool,
    /// Use pointer offset.
    pub use_offset: Option<usize>,
    /// Touch pointer offset.
    pub touch_offset: Option<usize>,
    /// Pain pointer offset.
    pub pain_offset: Option<usize>,
    /// Die pointer offset.
    pub die_offset: Option<usize>,
    /// Whether callbacks use the rerelease ABI.
    pub callback_rerelease: bool,
    /// Armor points field, if any.
    pub armor_points: Option<ScalarField>,
    /// Armor cells field, if any.
    pub armor_cells: Option<ScalarField>,
    /// Deferred blood field, if any.
    pub deferred_blood: Option<ScalarField>,
    /// Deferred knockback field, if any.
    pub deferred_knockback: Option<ScalarField>,
    /// Deferred receipt offset, if any.
    pub deferred_receipt: Option<usize>,
    /// Velocity vector offset.
    pub velocity_offset: usize,
}

/// Shared boundary operations the combat bridge needs.
pub trait CombatCalls {
    /// Whether an actor may take part in combat.
    fn eligible(&self, actor: NativeActorId) -> bool;
    /// Whether an actor is live.
    fn is_live(&self, actor: NativeActorId) -> bool;
    /// Resolve an entity address to its actor.
    fn actor_at(&self, address: GuestAddress) -> Result<NativeActorId, CombatError>;
    /// Resolve an actor to its entity address.
    fn address_of(&self, actor: NativeActorId) -> Result<GuestAddress, CombatError>;
    /// Owner of a slot, if bound.
    fn owner(&self, slot: usize) -> Option<NativeActorId>;
    /// Whether a damage target is owned by this mod.
    fn owns(&self, actor: NativeActorId) -> bool;
    /// Read a scalar at a record base.
    fn scalar(&self, base: GuestAddress, field: &ScalarField) -> Result<f64, CombatError>;
    /// Write a scalar at a record base.
    fn write_scalar(&self, base: GuestAddress, field: &ScalarField, value: f64) -> Result<(), CombatError>;
    /// Run a closure inside a transfer frame.
    fn transfer<R>(&self, invoke: impl FnOnce() -> R) -> R;
    /// Run a closure as source execution for an actor.
    fn source_execution<R>(&self, actor: NativeActorId, invoke: impl FnOnce() -> R) -> R;
    /// Flush shared projections after guest stores.
    fn synchronize(&self);
}

/// Shared actor operations for guest-originated callbacks.
pub trait SharedActorCallbacks {
    /// Apply damage whose target lives outside this mod.
    fn apply_foreign(&mut self, request: &DamageRequest);
    /// Route a guest use callback through shared state.
    fn source_use(
        &mut self,
        source: NativeActorId,
        other: Option<NativeActorId>,
        activator: Option<NativeActorId>,
        proceed: impl FnOnce(NativeActorId, Option<NativeActorId>, Option<NativeActorId>),
    );
    /// Route a guest touch callback through shared state.
    fn source_touch(&mut self, contact: TouchContact, proceed: impl FnOnce(TouchContact));
    /// Route a guest pain callback through shared state.
    fn source_pain(&mut self, reaction: PainReaction, proceed: impl FnOnce(PainReaction));
    /// Route a guest die callback through shared state.
    fn source_die(&mut self, reaction: DeathReaction, proceed: impl FnOnce(DeathReaction));
}

/// Guest body invoked by the synthetic host.
pub type GuestBody = Rc<dyn Fn(&SyntheticCombatHost, &[GuestCallValue]) -> GuestCallResult>;

struct CombatInner {
    memory: SparseGuestMemory,
    table_base: GuestAddress,
    stride: usize,
    count: usize,
    active: Vec<bool>,
    bodies: HashMap<u64, GuestBody>,
}

/// Headless combat host: entity table, guest memory, and guest bodies.
#[derive(Clone)]
pub struct SyntheticCombatHost {
    inner: Rc<RefCell<CombatInner>>,
}

impl SyntheticCombatHost {
    /// Build a host over a mapped entity table.
    pub fn new(memory: SparseGuestMemory, table_base: GuestAddress, stride: usize, count: usize) -> Self {
        Self {
            inner: Rc::new(RefCell::new(CombatInner {
                memory,
                table_base,
                stride,
                count,
                active: vec![false; count],
                bodies: HashMap::new(),
            })),
        }
    }

    /// Mark a slot active.
    pub fn set_active(&self, slot: usize, active: bool) {
        if let Some(flag) = self.inner.borrow_mut().active.get_mut(slot) {
            *flag = active;
        }
    }

    /// Register a guest body at an offset.
    pub fn register_body(
        &self,
        offset: u64,
        body: impl Fn(&SyntheticCombatHost, &[GuestCallValue]) -> GuestCallResult + 'static,
    ) {
        self.inner.borrow_mut().bodies.insert(offset, Rc::new(body));
    }

    /// Run a closure against guest memory.
    pub fn with_memory<R>(&self, run: impl FnOnce(&mut SparseGuestMemory) -> R) -> R {
        run(&mut self.inner.borrow_mut().memory)
    }

    /// Entity table geometry.
    pub fn table(&self) -> (GuestAddress, usize, usize) {
        let inner = self.inner.borrow();
        (inner.table_base, inner.stride, inner.count)
    }

    /// Pointer width in bytes.
    pub fn pointer_bytes(&self) -> usize {
        self.inner.borrow().memory.pointer_bytes()
    }

    /// Guest address-space token.
    pub fn address_space(&self) -> u64 {
        self.inner.borrow().memory.address_space()
    }

    /// Whether a slot is active.
    pub fn is_active(&self, slot: usize) -> bool {
        self.inner.borrow().active.get(slot).copied().unwrap_or(false)
    }

    /// Base address of one entity slot.
    pub fn entity_address(&self, slot: usize) -> Result<GuestAddress, CombatError> {
        let inner = self.inner.borrow();
        if slot >= inner.count {
            return Err(CombatError::ActorOutsideTable);
        }
        Ok(inner.memory.offset(inner.table_base, (slot * inner.stride) as i64)?)
    }

    /// Table slot for an entity address.
    pub fn slot_of(&self, address: GuestAddress) -> Result<usize, CombatError> {
        let inner = self.inner.borrow();
        if address.offset < inner.table_base.offset {
            return Err(CombatError::ActorOutsideTable);
        }
        let relative = address.offset - inner.table_base.offset;
        if relative % inner.stride as u64 != 0 || relative / inner.stride as u64 >= inner.count as u64 {
            return Err(CombatError::ActorOutsideTable);
        }
        Ok((relative / inner.stride as u64) as usize)
    }

    /// Invoke a registered guest body.
    pub fn invoke_body(&self, offset: u64, values: &[GuestCallValue]) -> Result<GuestCallResult, CombatError> {
        let body = self
            .inner
            .borrow()
            .bodies
            .get(&offset)
            .cloned()
            .ok_or(CombatError::MissingDamage)?;
        Ok(body(self, values))
    }
}

fn scalar_to_f64(bytes: &[u8], storage: GuestStorage) -> f64 {
    match storage {
        GuestStorage::Float32 => f64::from(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        GuestStorage::Float64 => {
            let mut word = [0u8; 8];
            word.copy_from_slice(&bytes[..8]);
            f64::from_le_bytes(word)
        }
        GuestStorage::Int8 => f64::from(bytes[0] as i8),
        GuestStorage::Uint8 => f64::from(bytes[0]),
        GuestStorage::Int16 => f64::from(i16::from_le_bytes([bytes[0], bytes[1]])),
        GuestStorage::Uint16 => f64::from(u16::from_le_bytes([bytes[0], bytes[1]])),
        GuestStorage::Int32 => f64::from(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        GuestStorage::Uint32 => f64::from(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
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

fn mod_layout(pointer_bytes: usize) -> GuestLayout {
    GuestLayout::new("q2:mod_t", 3, 1, pointer_bytes, Vec::new())
}

fn as_i32(value: Option<&GuestCallValue>) -> Result<i32, CombatError> {
    match value {
        Some(GuestCallValue::Int32(value)) => Ok(*value),
        Some(GuestCallValue::Uint32(value)) => Ok(*value as i32),
        Some(GuestCallValue::Int64(value)) => Ok(*value as i32),
        Some(GuestCallValue::Uint64(value)) => Ok(*value as i32),
        Some(GuestCallValue::Float32(value)) => Ok(*value as i32),
        Some(GuestCallValue::Float64(value)) => Ok(*value as i32),
        _ => Err(CombatError::BadActorArgument),
    }
}

fn as_f32(value: Option<&GuestCallValue>) -> Result<f32, CombatError> {
    match value {
        Some(GuestCallValue::Float32(value)) => Ok(*value),
        Some(GuestCallValue::Float64(value)) => Ok(*value as f32),
        Some(GuestCallValue::Int32(value)) => Ok(*value as f32),
        _ => Err(CombatError::BadActorArgument),
    }
}

fn vec_to_bytes(value: Vec3) -> [u8; 12] {
    let mut bytes = [0u8; 12];
    bytes[..4].copy_from_slice(&value.x.to_le_bytes());
    bytes[4..8].copy_from_slice(&value.y.to_le_bytes());
    bytes[8..].copy_from_slice(&value.z.to_le_bytes());
    bytes
}

#[derive(Debug, Clone)]
struct DamageFrame {
    request: DamageRequest,
    result: Option<DamageOutcome>,
}

/// Saved deferred batch.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedCombatDeferred {
    /// Damage target.
    pub target: SavedActorId,
    /// Retained request.
    pub request: DamageRequest,
    /// Accumulated blood.
    pub blood: f64,
    /// Accumulated knockback.
    pub knockback: f64,
}

/// Token for a bound actor slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundActor {
    /// Entity slot.
    pub slot: usize,
    /// Bound actor.
    pub actor: NativeActorId,
}

/// Native combat bridge over a synthetic host.
pub struct NativeModCombat<C: CombatCalls, S: SharedActorCallbacks> {
    definition: CombatDefinition,
    host: SyntheticCombatHost,
    calls: C,
    shared: S,
    entries: HashMap<u64, CallbackKind>,
    frames: Vec<DamageFrame>,
    pending_deferred: HashMap<NativeActorId, DamageRequest>,
    events: Vec<CombatEvent>,
    watched: Option<(u64, usize)>,
    suspended: bool,
    scratch: Vec<(GuestAddress, usize)>,
}

impl<C: CombatCalls, S: SharedActorCallbacks> NativeModCombat<C, S> {
    /// Build the bridge, validating the declaration.
    pub fn new(
        definition: CombatDefinition,
        host: SyntheticCombatHost,
        calls: C,
        shared: S,
    ) -> Result<Self, CombatError> {
        let mut bridge = Self {
            definition,
            host,
            calls,
            shared,
            entries: HashMap::new(),
            frames: Vec::new(),
            pending_deferred: HashMap::new(),
            events: Vec::new(),
            watched: None,
            suspended: false,
            scratch: Vec::new(),
        };
        bridge.validate()?;
        Ok(bridge)
    }

    /// Validate field bounds and ABI agreement.
    pub fn validate(&self) -> Result<(), CombatError> {
        let (_, stride, _) = self.host.table();
        let pointer_bytes = self.host.pointer_bytes();
        for kind in [
            CallbackKind::Use,
            CallbackKind::Touch,
            CallbackKind::Pain,
            CallbackKind::Die,
        ] {
            if let Some(offset) = self.offset(kind) {
                if offset + pointer_bytes > stride {
                    return Err(CombatError::CallbackFieldRange);
                }
            }
        }
        for field in [
            &self.definition.health,
            &self.definition.mass,
            &self.definition.takedamage,
            &self.definition.flags,
        ] {
            if field.offset + field.width(pointer_bytes) > stride {
                return Err(CombatError::CombatFieldRange);
            }
        }
        if let (Some(blood), Some(knockback), Some(receipt)) = (
            &self.definition.deferred_blood,
            &self.definition.deferred_knockback,
            self.definition.deferred_receipt,
        ) {
            for (offset, width) in [
                (blood.offset, blood.width(pointer_bytes)),
                (knockback.offset, knockback.width(pointer_bytes)),
                (receipt, 1),
            ] {
                if offset + width > stride {
                    return Err(CombatError::DeferredFieldRange);
                }
            }
        }
        if self.definition.velocity_offset + 12 > stride {
            return Err(CombatError::CombatFieldRange);
        }
        let classic = self.definition.damage_abi == DamageAbi::Classic;
        if classic != self.definition.causes_classic {
            return Err(CombatError::AbiMismatch);
        }
        Ok(())
    }

    fn offset(&self, kind: CallbackKind) -> Option<usize> {
        match kind {
            CallbackKind::Use => self.definition.use_offset,
            CallbackKind::Touch => self.definition.touch_offset,
            CallbackKind::Pain => self.definition.pain_offset,
            CallbackKind::Die => self.definition.die_offset,
        }
    }

    fn at(&self, slot: usize, offset: usize) -> Result<GuestAddress, CombatError> {
        let base = self.host.entity_address(slot)?;
        Ok(self.host.with_memory(|memory| memory.offset(base, offset as i64))?)
    }

    fn read_scalar_at(&self, slot: usize, field: &ScalarField) -> Result<f64, CombatError> {
        let base = self.host.entity_address(slot)?;
        self.calls.scalar(base, field)
    }

    fn write_scalar_at(&self, slot: usize, field: &ScalarField, value: f64) -> Result<(), CombatError> {
        let base = self.host.entity_address(slot)?;
        self.calls.write_scalar(base, field, value)
    }

    fn read_vector_at(&self, slot: usize, offset: usize) -> Result<Vec3, CombatError> {
        let address = self.at(slot, offset)?;
        Ok(self.host.with_memory(|memory| memory.read_f32x3(address))?)
    }

    /// Write a vector at a slot-relative offset.
    pub fn write_vector_at(&self, slot: usize, offset: usize, value: Vec3) -> Result<(), CombatError> {
        let address = self.at(slot, offset)?;
        let bytes = vec_to_bytes(value);
        Ok(self.host.with_memory(|memory| memory.write(address, &bytes))?)
    }

    fn read_pointer_at(&self, slot: usize, offset: usize) -> Result<Option<u64>, CombatError> {
        let address = self.at(slot, offset)?;
        Ok(self
            .host
            .with_memory(|memory| memory.read_pointer(address))?
            .map(|pointer| pointer.offset))
    }

    fn eligible_all(&self, actors: &[Option<NativeActorId>]) -> bool {
        actors
            .iter()
            .all(|actor| actor.is_none_or(|actor| self.calls.eligible(actor)))
    }

    fn nullable(&self, value: Option<&GuestCallValue>) -> Result<Option<NativeActorId>, CombatError> {
        let Some(GuestCallValue::Pointer(address)) = value else {
            return Err(CombatError::BadActorArgument);
        };
        (*address).map(|address| self.calls.actor_at(address)).transpose()
    }

    fn pointer_arg(&self, actor: Option<NativeActorId>) -> Result<GuestCallValue, CombatError> {
        match actor {
            None => Ok(GuestCallValue::Pointer(None)),
            Some(actor) => Ok(GuestCallValue::Pointer(Some(self.calls.address_of(actor)?))),
        }
    }

    /// Bind one owned slot, rescanning its callback pointers.
    pub fn bind(&mut self, slot: usize, actor: NativeActorId) -> Result<BoundActor, CombatError> {
        self.validate()?;
        self.watch();
        for kind in [
            CallbackKind::Use,
            CallbackKind::Touch,
            CallbackKind::Pain,
            CallbackKind::Die,
        ] {
            self.ensure(kind, slot)?;
        }
        Ok(BoundActor { slot, actor })
    }

    fn watch(&mut self) {
        let (base, stride, count) = self.host.table();
        self.watched = Some((base.offset, stride * count));
    }

    /// Watched table span in bytes, if observation is armed.
    #[must_use]
    pub fn watched_bytes(&self) -> Option<usize> {
        self.watched.map(|(_, bytes)| bytes)
    }

    /// Lazily bind the callback entry stored at a slot, if any.
    pub fn ensure(&mut self, kind: CallbackKind, slot: usize) -> Result<Option<u64>, CombatError> {
        let Some(offset) = self.offset(kind) else {
            return Ok(None);
        };
        let Some(address) = self.read_pointer_at(slot, offset)? else {
            return Ok(None);
        };
        if let Some(prior) = self.entries.get(&address) {
            if *prior != kind {
                return Err(CombatError::EntryConflict);
            }
            return Ok(Some(address));
        }
        self.entries.insert(address, kind);
        Ok(Some(address))
    }

    /// React to entity-relative writes: deferred capture plus pointer rescan.
    pub fn note_writes(
        &mut self,
        slot: usize,
        byte_offset: usize,
        byte_length: usize,
        current: Option<&DamageRequest>,
    ) -> Result<(), CombatError> {
        if self.suspended || self.calls.owner(slot).is_none() {
            return Ok(());
        }
        if let (Some(blood), Some(receipt)) = (self.definition.deferred_blood.clone(), self.definition.deferred_receipt)
        {
            let pointer_bytes = self.host.pointer_bytes();
            let hits_receipt = byte_offset < receipt + 1 && receipt < byte_offset + byte_length;
            if hits_receipt && self.read_scalar_at(slot, &blood)? != 0.0 {
                let Some(request) = current else {
                    return Err(CombatError::MissingProvenance);
                };
                self.pending_deferred.insert(request.target, request.clone());
            }
            let width = blood.width(pointer_bytes);
            let hits_blood = byte_offset < blood.offset + width && blood.offset < byte_offset + byte_length;
            if hits_blood && self.read_scalar_at(slot, &blood)? == 0.0 {
                if let Some(owner) = self.calls.owner(slot) {
                    self.pending_deferred.remove(&owner);
                }
            }
        }
        let pointer_bytes = self.host.pointer_bytes();
        for kind in [
            CallbackKind::Use,
            CallbackKind::Touch,
            CallbackKind::Pain,
            CallbackKind::Die,
        ] {
            if let Some(offset) = self.offset(kind) {
                if byte_offset < offset + pointer_bytes && offset < byte_offset + byte_length {
                    self.ensure(kind, slot)?;
                }
            }
        }
        Ok(())
    }

    /// Read the combat body state for one slot.
    pub fn read_combat_state(&self, slot: usize) -> Result<CombatBodyState, CombatError> {
        let flags = self.read_scalar_at(slot, &self.definition.flags)? as i64;
        Ok(CombatBodyState {
            health: self.read_scalar_at(slot, &self.definition.health)?,
            mass: self.read_scalar_at(slot, &self.definition.mass)?,
            armor: self.read_armor(slot)?,
            can_take_damage: self.read_scalar_at(slot, &self.definition.takedamage)? != 0.0,
            invulnerable: flags & self.definition.invulnerable_mask != 0,
            no_knockback: flags & self.definition.no_knockback_mask != 0,
        })
    }

    /// Write health for one slot.
    pub fn write_health(&self, slot: usize, value: f64) -> Result<(), CombatError> {
        let health = self.definition.health.clone();
        self.write_scalar_at(slot, &health, value)
    }

    fn read_armor(&self, slot: usize) -> Result<CombatArmor, CombatError> {
        let points = match &self.definition.armor_points {
            Some(field) => self.read_scalar_at(slot, field)?,
            None => 0.0,
        };
        let cells = match &self.definition.armor_cells {
            Some(field) => self.read_scalar_at(slot, field)?,
            None => 0.0,
        };
        Ok(CombatArmor { points, cells })
    }

    /// Write armor counters for one slot.
    pub fn write_armor(&self, slot: usize, armor: CombatArmor) -> Result<(), CombatError> {
        if let Some(field) = self.definition.armor_points.clone() {
            self.write_scalar_at(slot, &field, armor.points)?;
        }
        if let Some(field) = self.definition.armor_cells.clone() {
            self.write_scalar_at(slot, &field, armor.cells)?;
        }
        Ok(())
    }

    fn native_cause(&self, value: Option<&GuestCallValue>) -> Result<NativeCause, CombatError> {
        if self.definition.causes_classic {
            let Some(GuestCallValue::Int32(value)) = value else {
                return Err(CombatError::BadClassicCause);
            };
            return Ok(NativeCause::Classic(*value));
        }
        let Some(GuestCallValue::Aggregate { bytes, .. }) = value else {
            return Err(CombatError::BadRereleaseCause);
        };
        if bytes.len() != 3 {
            return Err(CombatError::BadRereleaseCause);
        }
        Ok(NativeCause::Rerelease {
            id: bytes[0],
            friendly_fire: bytes[1] != 0,
            no_point_loss: bytes[2] != 0,
        })
    }

    fn cause_value(&self, cause: NativeCause) -> Result<GuestCallValue, CombatError> {
        match (self.definition.damage_abi, cause) {
            (DamageAbi::Classic, NativeCause::Classic(value)) => Ok(GuestCallValue::Int32(value)),
            (
                DamageAbi::Rerelease,
                NativeCause::Rerelease {
                    id,
                    friendly_fire,
                    no_point_loss,
                },
            ) => Ok(GuestCallValue::Aggregate {
                layout: mod_layout(self.host.pointer_bytes()),
                bytes: vec![id, u8::from(friendly_fire), u8::from(no_point_loss)],
            }),
            _ => Err(CombatError::MissingMod),
        }
    }

    fn read_vector_arg(&self, value: Option<&GuestCallValue>) -> Result<Vec3, CombatError> {
        let Some(GuestCallValue::Pointer(Some(address))) = value else {
            return Err(CombatError::BadActorArgument);
        };
        let address = *address;
        Ok(self.host.with_memory(|memory| memory.read_f32x3(address))?)
    }

    /// Parse guest damage arguments into a canonical request.
    pub fn parse_request(&self, args: &[GuestCallValue]) -> Result<DamageRequest, CombatError> {
        let target = self.nullable(args.get(0))?.ok_or(CombatError::BadActorArgument)?;
        let inflictor = self.nullable(args.get(1))?;
        let attacker = self.nullable(args.get(2))?;
        let flags = as_i32(args.get(8))?;
        Ok(DamageRequest {
            target,
            attacker,
            inflictor,
            amount: f64::from(as_i32(args.get(6))?),
            knockback: f64::from(as_i32(args.get(7))?),
            direction: self.read_vector_arg(args.get(3))?,
            point: self.read_vector_arg(args.get(4))?,
            normal: self.read_vector_arg(args.get(5))?,
            delivery: if flags & 1 != 0 {
                Delivery::Radius
            } else {
                Delivery::Direct
            },
            cause: self.native_cause(args.get(9))?,
            damage_flags: flags,
        })
    }

    /// Damage entry interceptor: eligible targets apply locally.
    pub fn call_damage(&mut self, args: &[GuestCallValue]) -> Result<GuestCallResult, CombatError> {
        let request = self.parse_request(args)?;
        if !self.eligible_all(&[Some(request.target), request.attacker, request.inflictor]) {
            return Ok(GuestCallResult::Void);
        }
        if self.calls.owns(request.target) {
            self.apply(&request)?;
        } else {
            self.calls.transfer(|| {});
            self.shared.apply_foreign(&request);
            self.calls.synchronize();
        }
        Ok(GuestCallResult::Void)
    }

    /// Apply damage through the guest damage body with observation.
    pub fn apply(&mut self, request: &DamageRequest) -> Result<DamageOutcome, CombatError> {
        let Some(damage) = self.definition.damage_entry else {
            return Err(CombatError::MissingDamage);
        };
        if !self.eligible_all(&[Some(request.target), request.attacker, request.inflictor]) {
            return Ok(DamageOutcome {
                reaction: Reaction::None,
                applied: 0.0,
            });
        }
        let slot = {
            let address = self.calls.address_of(request.target)?;
            self.host.slot_of(address)?
        };
        let health_field = self.definition.health.clone();
        let velocity_offset = self.definition.velocity_offset;
        let before_health = self.read_scalar_at(slot, &health_field)?;
        let before_armor = self.read_armor(slot)?;
        let before_velocity = self.read_vector_at(slot, velocity_offset)?;
        self.frames.push(DamageFrame {
            request: request.clone(),
            result: None,
        });
        let args = self.marshal_damage(request)?;
        let host = self.host.clone();
        self.calls.transfer(|| host.invoke_body(damage, &args))?;
        self.release_scratch();
        let frame = self.frames.pop().unwrap_or(DamageFrame {
            request: request.clone(),
            result: None,
        });
        let after_health = self.read_scalar_at(slot, &health_field)?;
        let after_armor = self.read_armor(slot)?;
        let after_velocity = self.read_vector_at(slot, velocity_offset)?;
        if before_health != after_health {
            self.events.push(CombatEvent::Health {
                before: before_health,
                after: after_health,
            });
        }
        if before_armor != after_armor {
            self.events.push(CombatEvent::Armor {
                before: before_armor,
                after: after_armor,
            });
        }
        if before_velocity != after_velocity {
            self.events.push(CombatEvent::Velocity {
                before: before_velocity,
                after: after_velocity,
            });
        }
        if let Some(result) = frame.result {
            return Ok(result);
        }
        let applied = (before_health - after_health).max(0.0);
        let reaction = if applied > 0.0 && after_health <= 0.0 {
            Reaction::Death
        } else if applied > 0.0 {
            Reaction::Pain
        } else {
            Reaction::None
        };
        Ok(DamageOutcome { reaction, applied })
    }

    fn marshal_damage(&mut self, request: &DamageRequest) -> Result<Vec<GuestCallValue>, CombatError> {
        let target = self.calls.address_of(request.target)?;
        let inflictor = self.pointer_arg(request.inflictor)?;
        let attacker = self.pointer_arg(request.attacker)?;
        let vectors = [request.direction, request.point, request.normal];
        let mut pointers = Vec::with_capacity(3);
        for vector in vectors {
            let bytes = vec_to_bytes(vector);
            let address = Self::alloc_scratch_in(&self.host, &mut self.scratch, &bytes)?;
            pointers.push(GuestCallValue::Pointer(Some(address)));
        }
        let cause = self.cause_value(request.cause)?;
        Ok(vec![
            GuestCallValue::Pointer(Some(target)),
            inflictor,
            attacker,
            pointers[0].clone(),
            pointers[1].clone(),
            pointers[2].clone(),
            GuestCallValue::Int32(request.amount.trunc() as i32),
            GuestCallValue::Int32(request.knockback.trunc() as i32),
            GuestCallValue::Int32(request.damage_flags),
            cause,
        ])
    }

    /// Record a source reaction for the innermost matching frame.
    pub fn note_source_reaction(&mut self, target: NativeActorId, reaction: Reaction, applied: f64) {
        if let Some(frame) = self.frames.last_mut() {
            if frame.request.target == target && frame.result.is_none() {
                frame.result = Some(DamageOutcome { reaction, applied });
            }
        }
    }

    /// Fire a use callback toward the guest body.
    pub fn fire_use(
        &mut self,
        bound: BoundActor,
        other: Option<NativeActorId>,
        activator: Option<NativeActorId>,
    ) -> Result<(), CombatError> {
        if !self.eligible_all(&[Some(bound.actor), other, activator]) {
            return Ok(());
        }
        let Some(entry) = self.ensure(CallbackKind::Use, bound.slot)? else {
            return Ok(());
        };
        let args = vec![
            self.pointer_arg(Some(bound.actor))?,
            self.pointer_arg(other)?,
            self.pointer_arg(activator)?,
        ];
        let host = self.host.clone();
        let actor = bound.actor;
        self.calls
            .transfer(|| self.calls.source_execution(actor, || host.invoke_body(entry, &args)))?;
        Ok(())
    }

    /// Fire a touch callback toward the guest body.
    pub fn fire_touch(&mut self, contact: &TouchContact) -> Result<(), CombatError> {
        let slot = {
            let address = self.calls.address_of(contact.source)?;
            self.host.slot_of(address)?
        };
        if !self.eligible_all(&[Some(contact.source), Some(contact.other)]) {
            return Ok(());
        }
        let Some(entry) = self.ensure(CallbackKind::Touch, slot)? else {
            return Ok(());
        };
        let args = self.marshal_touch(contact)?;
        let host = self.host.clone();
        let actor = contact.source;
        let invoked = self
            .calls
            .transfer(|| self.calls.source_execution(actor, || host.invoke_body(entry, &args)));
        self.release_scratch();
        invoked?;
        Ok(())
    }

    /// Fire a pain callback toward the guest body.
    pub fn fire_pain(&mut self, reaction: &PainReaction) -> Result<(), CombatError> {
        let slot = {
            let address = self.calls.address_of(reaction.source)?;
            self.host.slot_of(address)?
        };
        if !self.eligible_all(&[Some(reaction.source), reaction.attacker]) {
            return Ok(());
        }
        let Some(entry) = self.ensure(CallbackKind::Pain, slot)? else {
            return Ok(());
        };
        let args = self.marshal_pain(reaction)?;
        let host = self.host.clone();
        let actor = reaction.source;
        let invoked = self
            .calls
            .transfer(|| self.calls.source_execution(actor, || host.invoke_body(entry, &args)));
        self.release_scratch();
        invoked?;
        Ok(())
    }

    /// Fire a die callback toward the guest body.
    pub fn fire_die(&mut self, reaction: &DeathReaction) -> Result<(), CombatError> {
        let slot = {
            let address = self.calls.address_of(reaction.source)?;
            self.host.slot_of(address)?
        };
        if !self.eligible_all(&[Some(reaction.source), reaction.attacker, reaction.inflictor]) {
            return Ok(());
        }
        let Some(entry) = self.ensure(CallbackKind::Die, slot)? else {
            return Ok(());
        };
        let args = self.marshal_die(reaction)?;
        let host = self.host.clone();
        let actor = reaction.source;
        let invoked = self
            .calls
            .transfer(|| self.calls.source_execution(actor, || host.invoke_body(entry, &args)));
        self.release_scratch();
        invoked?;
        Ok(())
    }

    fn alloc_scratch_in(
        host: &SyntheticCombatHost,
        scratch: &mut Vec<(GuestAddress, usize)>,
        bytes: &[u8],
    ) -> Result<GuestAddress, CombatError> {
        let address = host.with_memory(|memory| {
            memory.allocate(&qa_guest::core::contracts::GuestAllocationOptions::bytes(bytes.len()))
        })?;
        host.with_memory(|memory| memory.write(address, bytes))?;
        scratch.push((address, bytes.len()));
        Ok(address)
    }

    fn alloc_scratch(&mut self, bytes: &[u8]) -> Result<GuestAddress, CombatError> {
        Self::alloc_scratch_in(&self.host, &mut self.scratch, bytes)
    }

    fn release_scratch(&mut self) {
        let scratch = std::mem::take(&mut self.scratch);
        self.host.with_memory(|memory| {
            for (address, length) in scratch {
                let _ = memory.unmap(address, length);
            }
        });
    }

    fn marshal_touch(&mut self, contact: &TouchContact) -> Result<Vec<GuestCallValue>, CombatError> {
        Self::marshal_touch_args(
            &self.host,
            &self.calls,
            self.definition.callback_rerelease,
            &mut self.scratch,
            contact,
        )
    }

    fn marshal_touch_args(
        host: &SyntheticCombatHost,
        calls: &C,
        rerelease: bool,
        scratch: &mut Vec<(GuestAddress, usize)>,
        contact: &TouchContact,
    ) -> Result<Vec<GuestCallValue>, CombatError> {
        let pointer_arg = |actor: Option<NativeActorId>| -> Result<GuestCallValue, CombatError> {
            match actor {
                None => Ok(GuestCallValue::Pointer(None)),
                Some(actor) => Ok(GuestCallValue::Pointer(Some(calls.address_of(actor)?))),
            }
        };
        if rerelease {
            let trace = contact.trace.as_ref().ok_or(CombatError::MissingTrace)?;
            let bytes = Self::encode_trace_args(host, calls, scratch, trace)?;
            let address = Self::alloc_scratch_in(host, scratch, &bytes)?;
            return Ok(vec![
                pointer_arg(Some(contact.source))?,
                pointer_arg(Some(contact.other))?,
                GuestCallValue::Pointer(Some(address)),
                GuestCallValue::Uint32(u32::from(trace.inverted)),
            ]);
        }
        let mut plane = [0u8; 20];
        if let Some(value) = &contact.plane {
            plane[..12].copy_from_slice(&vec_to_bytes(value.normal));
            plane[12..16].copy_from_slice(&value.distance.to_le_bytes());
            plane[16] = value.plane_type;
            plane[17] = value.signbits;
        }
        let plane_address = Self::alloc_scratch_in(host, scratch, &plane)?;
        let mut surface = [0u8; 24];
        if let Some(value) = &contact.surface {
            let name = value.name.as_bytes();
            surface[..name.len().min(16)].copy_from_slice(&name[..name.len().min(16)]);
            surface[16..20].copy_from_slice(&value.flags.to_le_bytes());
            surface[20..24].copy_from_slice(&value.value.to_le_bytes());
        }
        let surface_address = Self::alloc_scratch_in(host, scratch, &surface)?;
        Ok(vec![
            pointer_arg(Some(contact.source))?,
            pointer_arg(Some(contact.other))?,
            GuestCallValue::Pointer(contact.plane.map(|_| plane_address)),
            GuestCallValue::Pointer(contact.surface.as_ref().map(|_| surface_address)),
        ])
    }

    fn encode_trace_args(
        host: &SyntheticCombatHost,
        calls: &C,
        scratch: &mut Vec<(GuestAddress, usize)>,
        trace: &TraceSnapshot,
    ) -> Result<Vec<u8>, CombatError> {
        let mut bytes = vec![0u8; 96];
        bytes[0] = u8::from(trace.all_solid);
        bytes[1] = u8::from(trace.start_solid);
        bytes[4..8].copy_from_slice(&trace.fraction.to_le_bytes());
        bytes[8..20].copy_from_slice(&vec_to_bytes(trace.end));
        bytes[48..52].copy_from_slice(&trace.contents.to_le_bytes());
        if let Some(surface) = &trace.surface {
            let mut blob = vec![0u8; 60];
            let name = surface.name.as_bytes();
            blob[..name.len().min(32)].copy_from_slice(&name[..name.len().min(32)]);
            blob[32..36].copy_from_slice(&surface.flags.to_le_bytes());
            blob[36..40].copy_from_slice(&surface.value.to_le_bytes());
            let address = Self::alloc_scratch_in(host, scratch, &blob)?;
            let pointer = address.offset as u32;
            bytes[40..44].copy_from_slice(&pointer.to_le_bytes());
        }
        let ent = calls.address_of(trace.hit_actor)?;
        bytes[56..60].copy_from_slice(&(ent.offset as u32).to_le_bytes());
        Ok(bytes)
    }

    fn marshal_pain(&mut self, reaction: &PainReaction) -> Result<Vec<GuestCallValue>, CombatError> {
        let mut args = vec![
            self.pointer_arg(Some(reaction.source))?,
            self.pointer_arg(reaction.attacker)?,
            GuestCallValue::Float32(reaction.kick),
            GuestCallValue::Int32(reaction.damage.trunc() as i32),
        ];
        if self.definition.callback_rerelease {
            let cause = reaction.cause.ok_or(CombatError::MissingMod)?;
            args.push(self.mod_pointer(cause)?);
        }
        Ok(args)
    }

    fn marshal_die(&mut self, reaction: &DeathReaction) -> Result<Vec<GuestCallValue>, CombatError> {
        let point = vec_to_bytes(reaction.point);
        let address = self.alloc_scratch(&point)?;
        let mut args = vec![
            self.pointer_arg(Some(reaction.source))?,
            self.pointer_arg(reaction.inflictor)?,
            self.pointer_arg(reaction.attacker)?,
            GuestCallValue::Int32(reaction.damage.trunc() as i32),
            GuestCallValue::Pointer(Some(address)),
        ];
        if self.definition.callback_rerelease {
            let cause = reaction.cause.ok_or(CombatError::MissingMod)?;
            args.push(self.mod_pointer(cause)?);
        }
        Ok(args)
    }

    fn mod_pointer(&mut self, cause: NativeCause) -> Result<GuestCallValue, CombatError> {
        let NativeCause::Rerelease {
            id,
            friendly_fire,
            no_point_loss,
        } = cause
        else {
            return Err(CombatError::MissingMod);
        };
        let address = self.alloc_scratch(&[id, u8::from(friendly_fire), u8::from(no_point_loss)])?;
        Ok(GuestCallValue::Pointer(Some(address)))
    }

    /// Parse a guest touch call into a contact.
    pub fn parse_touch(&self, source: NativeActorId, args: &[GuestCallValue]) -> Result<TouchContact, CombatError> {
        let other = self.nullable(args.get(1))?.ok_or(CombatError::BadActorArgument)?;
        if self.definition.callback_rerelease {
            let Some(GuestCallValue::Pointer(Some(address))) = args.get(2) else {
                return Err(CombatError::BadTouchArgs);
            };
            let bytes = self.host.with_memory(|memory| memory.copy(*address, 96))?;
            let ent_raw = u32::from_le_bytes([bytes[56], bytes[57], bytes[58], bytes[59]]);
            let ent = if ent_raw == 0 {
                other
            } else {
                let address = GuestAddress::new(self.host.address_space(), u64::from(ent_raw));
                self.calls.actor_at(address)?
            };
            let surface_raw = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]);
            let surface = if surface_raw == 0 {
                None
            } else {
                let address = GuestAddress::new(self.host.address_space(), u64::from(surface_raw));
                let blob = self.host.with_memory(|memory| memory.copy(address, 60))?;
                Some(ContactSurface {
                    name: String::from_utf8_lossy(&blob[..32]).trim_end_matches('\0').to_string(),
                    flags: i32::from_le_bytes([blob[32], blob[33], blob[34], blob[35]]),
                    value: i32::from_le_bytes([blob[36], blob[37], blob[38], blob[39]]),
                })
            };
            let mut end = [0u8; 12];
            end.copy_from_slice(&bytes[8..20]);
            return Ok(TouchContact {
                source,
                other,
                plane: None,
                surface: surface.clone(),
                trace: Some(TraceSnapshot {
                    all_solid: bytes[0] != 0,
                    start_solid: bytes[1] != 0,
                    fraction: f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
                    end: Vec3 {
                        x: f32::from_le_bytes([end[0], end[1], end[2], end[3]]),
                        y: f32::from_le_bytes([end[4], end[5], end[6], end[7]]),
                        z: f32::from_le_bytes([end[8], end[9], end[10], end[11]]),
                    },
                    contents: u32::from_le_bytes([bytes[48], bytes[49], bytes[50], bytes[51]]),
                    surface,
                    hit_actor: ent,
                    inverted: as_i32(args.get(3))? != 0,
                }),
            });
        }
        let (Some(plane), Some(surface)) = (args.get(2), args.get(3)) else {
            return Err(CombatError::BadTouchArgs);
        };
        let plane = match plane {
            GuestCallValue::Pointer(None) => None,
            GuestCallValue::Pointer(Some(address)) => {
                let bytes = self.host.with_memory(|memory| memory.copy(*address, 20))?;
                Some(ContactPlane {
                    normal: Vec3 {
                        x: f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
                        y: f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
                        z: f32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
                    },
                    distance: f32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
                    plane_type: bytes[16],
                    signbits: bytes[17],
                })
            }
            _ => return Err(CombatError::BadTouchArgs),
        };
        let surface = match surface {
            GuestCallValue::Pointer(None) => None,
            GuestCallValue::Pointer(Some(address)) => {
                let bytes = self.host.with_memory(|memory| memory.copy(*address, 24))?;
                Some(ContactSurface {
                    name: String::from_utf8_lossy(&bytes[..16]).trim_end_matches('\0').to_string(),
                    flags: i32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]),
                    value: i32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]),
                })
            }
            _ => return Err(CombatError::BadTouchArgs),
        };
        Ok(TouchContact {
            source,
            other,
            plane,
            surface,
            trace: None,
        })
    }

    /// Route a guest use call through shared state.
    pub fn source_use(
        &mut self,
        source: NativeActorId,
        args: &[GuestCallValue],
        original: impl FnOnce(&[GuestCallValue]) -> GuestCallResult,
    ) -> Result<GuestCallResult, CombatError> {
        let other = self.nullable(args.get(1))?;
        let activator = self.nullable(args.get(2))?;
        let mut result = GuestCallResult::Void;
        let mut outcome: Result<(), CombatError> = Ok(());
        let calls = &self.calls;
        self.shared
            .source_use(source, other, activator, |actor, next_other, next_activator| {
                if actor != source {
                    outcome = Err(CombatError::Retargeted);
                    return;
                }
                if ![Some(actor), next_other, next_activator]
                    .iter()
                    .all(|actor| actor.is_none_or(|actor| calls.eligible(actor)))
                {
                    return;
                }
                calls.synchronize();
                result = if next_other == other && next_activator == activator {
                    original(args)
                } else {
                    let rebuilt = [
                        GuestCallValue::Pointer(calls.address_of(actor).ok()),
                        GuestCallValue::Pointer(next_other.and_then(|actor| calls.address_of(actor).ok())),
                        GuestCallValue::Pointer(next_activator.and_then(|actor| calls.address_of(actor).ok())),
                    ];
                    original(&rebuilt)
                };
            });
        outcome?;
        Ok(result)
    }

    /// Route a guest touch call through shared state.
    pub fn source_touch(
        &mut self,
        source: NativeActorId,
        args: &[GuestCallValue],
        original: impl FnOnce(&[GuestCallValue]) -> GuestCallResult,
    ) -> Result<GuestCallResult, CombatError> {
        let contact = self.parse_touch(source, args)?;
        let mut result = GuestCallResult::Void;
        let mut outcome: Result<(), CombatError> = Ok(());
        let rerelease = self.definition.callback_rerelease;
        let (host, calls, scratch) = (&self.host, &self.calls, &mut self.scratch);
        self.shared.source_touch(contact.clone(), |effective| {
            if effective.source != source {
                outcome = Err(CombatError::Retargeted);
                return;
            }
            if ![Some(effective.source), Some(effective.other)]
                .iter()
                .all(|actor| actor.is_none_or(|actor| calls.eligible(actor)))
            {
                return;
            }
            calls.synchronize();
            result = if effective == contact {
                original(args)
            } else {
                match Self::marshal_touch_args(host, calls, rerelease, scratch, &effective) {
                    Ok(rebuilt) => original(&rebuilt),
                    Err(error) => {
                        outcome = Err(error);
                        return;
                    }
                }
            };
        });
        outcome?;
        self.release_scratch();
        Ok(result)
    }

    /// Route a guest pain call through shared state.
    pub fn source_pain(
        &mut self,
        source: NativeActorId,
        args: &[GuestCallValue],
        original: impl FnOnce(&[GuestCallValue]) -> GuestCallResult,
    ) -> Result<GuestCallResult, CombatError> {
        let attacker = self.nullable(args.get(1))?;
        let damage = f64::from(as_i32(args.get(3))?);
        self.note_source_reaction(source, Reaction::Pain, damage);
        let cause = self.reaction_cause(source, args, 4)?;
        let reaction = PainReaction {
            source,
            attacker,
            cause,
            kick: as_f32(args.get(2))?,
            damage,
        };
        let mut result = GuestCallResult::Void;
        let mut outcome: Result<(), CombatError> = Ok(());
        let calls = &self.calls;
        self.shared.source_pain(reaction.clone(), |effective| {
            if effective.source != source {
                outcome = Err(CombatError::Retargeted);
                return;
            }
            if ![Some(effective.source), effective.attacker]
                .iter()
                .all(|actor| actor.is_none_or(|actor| calls.eligible(actor)))
            {
                return;
            }
            calls.synchronize();
            result = original(args);
        });
        outcome?;
        Ok(result)
    }

    /// Route a guest die call through shared state.
    pub fn source_die(
        &mut self,
        source: NativeActorId,
        args: &[GuestCallValue],
        original: impl FnOnce(&[GuestCallValue]) -> GuestCallResult,
    ) -> Result<GuestCallResult, CombatError> {
        let inflictor = self.nullable(args.get(1))?;
        let attacker = self.nullable(args.get(2))?;
        let damage = f64::from(as_i32(args.get(3))?);
        self.note_source_reaction(source, Reaction::Death, damage);
        let cause = self.reaction_cause(source, args, 5)?;
        let reaction = DeathReaction {
            source,
            inflictor,
            attacker,
            cause,
            damage,
            point: self.read_vector_arg(args.get(4))?,
        };
        let mut result = GuestCallResult::Void;
        let mut outcome: Result<(), CombatError> = Ok(());
        let calls = &self.calls;
        self.shared.source_die(reaction.clone(), |effective| {
            if effective.source != source {
                outcome = Err(CombatError::Retargeted);
                return;
            }
            if ![Some(effective.source), effective.attacker, effective.inflictor]
                .iter()
                .all(|actor| actor.is_none_or(|actor| calls.eligible(actor)))
            {
                return;
            }
            calls.synchronize();
            result = original(args);
        });
        outcome?;
        Ok(result)
    }

    fn reaction_cause(
        &self,
        source: NativeActorId,
        args: &[GuestCallValue],
        mod_index: usize,
    ) -> Result<Option<NativeCause>, CombatError> {
        if let Some(frame) = self.frames.last() {
            if frame.request.target == source {
                return Ok(Some(frame.request.cause));
            }
        }
        if !self.definition.callback_rerelease {
            return Ok(None);
        }
        let Some(GuestCallValue::Pointer(Some(address))) = args.get(mod_index) else {
            return Ok(None);
        };
        let bytes = self.host.with_memory(|memory| memory.copy(*address, 3))?;
        Ok(Some(NativeCause::Rerelease {
            id: bytes[0],
            friendly_fire: bytes[1] != 0,
            no_point_loss: bytes[2] != 0,
        }))
    }

    /// Drain published combat events.
    pub fn drain_events(&mut self) -> Vec<CombatEvent> {
        std::mem::take(&mut self.events)
    }

    /// Drop deferred state for a released actor.
    pub fn release(&mut self, actor: NativeActorId) {
        self.pending_deferred.remove(&actor);
    }

    /// Capture deferred batches with their live accumulator counters.
    pub fn checkpoint_deferred(&self) -> Result<Vec<SavedCombatDeferred>, CombatError> {
        let mut records = Vec::with_capacity(self.pending_deferred.len());
        for (actor, request) in &self.pending_deferred {
            let base = self.calls.address_of(*actor)?;
            let blood = match &self.definition.deferred_blood {
                Some(field) => self.calls.scalar(base, field)?,
                None => request.amount,
            };
            let knockback = match &self.definition.deferred_knockback {
                Some(field) => self.calls.scalar(base, field)?,
                None => request.knockback,
            };
            records.push(SavedCombatDeferred {
                target: SavedActorId::from(*actor),
                request: request.clone(),
                blood,
                knockback,
            });
        }
        Ok(records)
    }

    /// Restore deferred batches after ownership checks.
    pub fn restore_deferred(&mut self, records: &[SavedCombatDeferred]) -> Result<(), CombatError> {
        for record in records {
            let target = NativeActorId {
                slot: record.target.slot,
                generation: record.target.generation,
            };
            if !self.calls.is_live(target) {
                return Err(CombatError::ForeignDeferred);
            }
        }
        self.pending_deferred.clear();
        for record in records {
            let target = NativeActorId {
                slot: record.target.slot,
                generation: record.target.generation,
            };
            self.pending_deferred.insert(target, record.request.clone());
        }
        Ok(())
    }

    /// Suspend or resume observation.
    pub fn suspend(&mut self, value: bool) {
        self.suspended = value;
        if value {
            self.watched = None;
        }
    }

    /// Release entries and deferred state.
    pub fn close(&mut self) {
        self.watched = None;
        self.entries.clear();
        self.pending_deferred.clear();
        self.release_scratch();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, GuestMapOptions, GuestPermissions, ModuleIdentity};
    use std::collections::HashSet;

    const STRIDE: usize = 128;
    const DAMAGE_ENTRY: u64 = 0x5000;
    const USE_ENTRY: u64 = 0x5100;
    const TOUCH_ENTRY: u64 = 0x5200;

    fn actor(slot: u32) -> NativeActorId {
        NativeActorId { slot, generation: 1 }
    }

    fn field(offset: usize) -> ScalarField {
        ScalarField {
            offset,
            storage: GuestStorage::Int32,
        }
    }

    fn definition() -> CombatDefinition {
        CombatDefinition {
            health: field(0),
            mass: field(4),
            takedamage: field(8),
            flags: field(12),
            invulnerable_mask: 1,
            no_knockback_mask: 2,
            damage_entry: Some(DAMAGE_ENTRY),
            damage_abi: DamageAbi::Classic,
            causes_classic: true,
            use_offset: Some(16),
            touch_offset: Some(40),
            pain_offset: None,
            die_offset: None,
            callback_rerelease: false,
            armor_points: Some(field(44)),
            armor_cells: Some(field(48)),
            deferred_blood: Some(field(20)),
            deferred_knockback: Some(field(24)),
            deferred_receipt: Some(28),
            velocity_offset: 32,
        }
    }

    struct TestCalls {
        host: SyntheticCombatHost,
        owners: HashMap<usize, NativeActorId>,
        eligible: HashSet<NativeActorId>,
        owned: HashSet<NativeActorId>,
        synchronizes: Rc<RefCell<u32>>,
    }

    impl TestCalls {
        fn scalar_bytes(value: f64, storage: GuestStorage) -> Vec<u8> {
            match storage {
                GuestStorage::Int32 => (value as i32).to_le_bytes().to_vec(),
                GuestStorage::Float32 => (value as f32).to_le_bytes().to_vec(),
                _ => (value as i32).to_le_bytes().to_vec(),
            }
        }
    }

    impl CombatCalls for TestCalls {
        fn eligible(&self, actor: NativeActorId) -> bool {
            self.eligible.contains(&actor)
        }

        fn is_live(&self, actor: NativeActorId) -> bool {
            self.eligible.contains(&actor)
        }

        fn actor_at(&self, address: GuestAddress) -> Result<NativeActorId, CombatError> {
            let slot = self.host.slot_of(address)?;
            self.owners.get(&slot).copied().ok_or(CombatError::ActorOutsideTable)
        }

        fn address_of(&self, actor: NativeActorId) -> Result<GuestAddress, CombatError> {
            let slot = self
                .owners
                .iter()
                .find(|(_, bound)| **bound == actor)
                .map(|(slot, _)| *slot)
                .ok_or(CombatError::ActorOutsideTable)?;
            self.host.entity_address(slot)
        }

        fn owner(&self, slot: usize) -> Option<NativeActorId> {
            self.owners.get(&slot).copied()
        }

        fn owns(&self, actor: NativeActorId) -> bool {
            self.owned.contains(&actor)
        }

        fn scalar(&self, base: GuestAddress, field: &ScalarField) -> Result<f64, CombatError> {
            let width = field.width(self.host.pointer_bytes());
            let bytes = self
                .host
                .with_memory(|memory| memory.copy(memory.offset(base, field.offset as i64)?, width))?;
            Ok(scalar_to_f64(&bytes, field.storage))
        }

        fn write_scalar(&self, base: GuestAddress, field: &ScalarField, value: f64) -> Result<(), CombatError> {
            let bytes = Self::scalar_bytes(value, field.storage);
            Ok(self
                .host
                .with_memory(|memory| memory.write(memory.offset(base, field.offset as i64)?, &bytes))?)
        }

        fn transfer<R>(&self, invoke: impl FnOnce() -> R) -> R {
            invoke()
        }

        fn source_execution<R>(&self, _actor: NativeActorId, invoke: impl FnOnce() -> R) -> R {
            invoke()
        }

        fn synchronize(&self) {
            *self.synchronizes.borrow_mut() += 1;
        }
    }

    struct TestShared {
        foreign: Rc<RefCell<Vec<NativeActorId>>>,
        routed: Rc<RefCell<Vec<String>>>,
    }

    impl SharedActorCallbacks for TestShared {
        fn apply_foreign(&mut self, request: &DamageRequest) {
            self.foreign.borrow_mut().push(request.target);
        }

        fn source_use(
            &mut self,
            source: NativeActorId,
            other: Option<NativeActorId>,
            activator: Option<NativeActorId>,
            proceed: impl FnOnce(NativeActorId, Option<NativeActorId>, Option<NativeActorId>),
        ) {
            self.routed.borrow_mut().push(format!("use:{}", source.slot));
            proceed(source, other, activator);
        }

        fn source_touch(&mut self, contact: TouchContact, proceed: impl FnOnce(TouchContact)) {
            self.routed.borrow_mut().push(format!("touch:{}", contact.source.slot));
            proceed(contact);
        }

        fn source_pain(&mut self, reaction: PainReaction, proceed: impl FnOnce(PainReaction)) {
            self.routed.borrow_mut().push(format!("pain:{}", reaction.source.slot));
            proceed(reaction);
        }

        fn source_die(&mut self, reaction: DeathReaction, proceed: impl FnOnce(DeathReaction)) {
            self.routed.borrow_mut().push(format!("die:{}", reaction.source.slot));
            proceed(reaction);
        }
    }

    struct Fixture {
        combat: NativeModCombat<TestCalls, TestShared>,
        host: SyntheticCombatHost,
        foreign: Rc<RefCell<Vec<NativeActorId>>>,
        routed: Rc<RefCell<Vec<String>>>,
        synchronizes: Rc<RefCell<u32>>,
    }

    fn fixture() -> Fixture {
        let module = ModuleIdentity::new(
            ProviderId::new("test", "combat"),
            "combat.so",
            ContentDigest {
                algorithm: "none".to_string(),
                value: "0".to_string(),
            },
            "r1",
        );
        let mut memory = SparseGuestMemory::new(module, 4, 0x90000).unwrap();
        let table_base = memory
            .map(&GuestMapOptions::new(0x20000, STRIDE * 8, GuestPermissions::ReadWrite))
            .unwrap();
        memory
            .map(&GuestMapOptions::new(0x5000, 0x1000, GuestPermissions::ReadWrite))
            .unwrap();
        memory
            .map(&GuestMapOptions::new(0x6000, 0x100, GuestPermissions::ReadWrite))
            .unwrap();
        let host = SyntheticCombatHost::new(memory, table_base, STRIDE, 8);
        host.set_active(1, true);
        host.set_active(2, true);
        host.set_active(3, true);

        let damage_host = host.clone();
        host.register_body(DAMAGE_ENTRY, move |_, values| {
            let target = match &values[0] {
                GuestCallValue::Pointer(Some(address)) => *address,
                _ => return GuestCallResult::Void,
            };
            let amount = match &values[6] {
                GuestCallValue::Int32(value) => *value,
                _ => 0,
            };
            let slot = damage_host.slot_of(target).unwrap();
            let base = damage_host.entity_address(slot).unwrap();
            damage_host.with_memory(|memory| {
                let address = memory.offset(base, 0).unwrap();
                let bytes = memory.copy(address, 4).unwrap();
                let health = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                memory.write(address, &(health - amount).to_le_bytes()).unwrap();
            });
            GuestCallResult::Void
        });

        let mut owners = HashMap::new();
        owners.insert(1, actor(1));
        owners.insert(2, actor(2));
        owners.insert(3, actor(3));
        let synchronizes = Rc::new(RefCell::new(0));
        let calls = TestCalls {
            host: host.clone(),
            owners,
            eligible: [actor(1), actor(2), actor(3)].into_iter().collect(),
            owned: [actor(1), actor(2)].into_iter().collect(),
            synchronizes: synchronizes.clone(),
        };
        let foreign = Rc::new(RefCell::new(Vec::new()));
        let routed = Rc::new(RefCell::new(Vec::new()));
        let shared = TestShared {
            foreign: foreign.clone(),
            routed: routed.clone(),
        };
        let combat = NativeModCombat::new(definition(), host.clone(), calls, shared).unwrap();
        Fixture {
            combat,
            host,
            foreign,
            routed,
            synchronizes,
        }
    }

    fn write_i32(host: &SyntheticCombatHost, slot: usize, offset: usize, value: i32) {
        let base = host.entity_address(slot).unwrap();
        host.with_memory(|memory| {
            let address = memory.offset(base, offset as i64).unwrap();
            memory.write(address, &value.to_le_bytes()).unwrap();
        });
    }

    fn damage_request(target: NativeActorId) -> DamageRequest {
        DamageRequest {
            target,
            attacker: Some(actor(2)),
            inflictor: None,
            amount: 30.0,
            knockback: 50.0,
            direction: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            point: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            normal: Vec3 { x: 0.0, y: 1.0, z: 0.0 },
            delivery: Delivery::Direct,
            cause: NativeCause::Classic(1),
            damage_flags: 0,
        }
    }

    #[test]
    fn damage_applies_with_observation_and_foreign_targets_forward() {
        let mut fixture = fixture();
        write_i32(&fixture.host, 1, 0, 100);
        write_i32(&fixture.host, 1, 8, 1);
        let outcome = fixture.combat.apply(&damage_request(actor(1))).unwrap();
        assert_eq!(
            outcome,
            DamageOutcome {
                reaction: Reaction::Pain,
                applied: 30.0
            }
        );
        let events = fixture.combat.drain_events();
        assert_eq!(
            events,
            vec![CombatEvent::Health {
                before: 100.0,
                after: 70.0
            }]
        );
        let state = fixture.combat.read_combat_state(1).unwrap();
        assert_eq!(state.health, 70.0);
        assert!(state.can_take_damage);

        let target = fixture.host.entity_address(3).unwrap();
        let data = GuestAddress::new(fixture.host.address_space(), 0x6000);
        let args = vec![
            GuestCallValue::Pointer(Some(target)),
            GuestCallValue::Pointer(None),
            GuestCallValue::Pointer(None),
            GuestCallValue::Pointer(Some(data)),
            GuestCallValue::Pointer(Some(
                fixture.host.with_memory(|memory| memory.offset(data, 12).unwrap()),
            )),
            GuestCallValue::Pointer(Some(
                fixture.host.with_memory(|memory| memory.offset(data, 24).unwrap()),
            )),
            GuestCallValue::Int32(10),
            GuestCallValue::Int32(0),
            GuestCallValue::Int32(0),
            GuestCallValue::Int32(2),
        ];
        let result = fixture.combat.call_damage(&args).unwrap();
        assert!(matches!(result, GuestCallResult::Void));
        assert_eq!(*fixture.foreign.borrow(), vec![actor(3)]);
        assert_eq!(*fixture.synchronizes.borrow(), 1);
    }

    #[test]
    fn use_callback_roundtrips_through_shared_state() {
        let mut fixture = fixture();
        let fired = Rc::new(RefCell::new(Vec::new()));
        let seen = fired.clone();
        fixture.host.register_body(USE_ENTRY, move |_, values| {
            seen.borrow_mut().push(values.len());
            GuestCallResult::Void
        });
        let base = fixture.host.entity_address(1).unwrap();
        let code = GuestAddress::new(fixture.host.address_space(), USE_ENTRY);
        fixture.host.with_memory(|memory| {
            let address = memory.offset(base, 16).unwrap();
            memory.write_pointer(address, Some(code)).unwrap();
        });
        let bound = fixture.combat.bind(1, actor(1)).unwrap();
        assert_eq!(fixture.combat.watched_bytes(), Some(STRIDE * 8));
        fixture.combat.fire_use(bound, None, Some(actor(2))).unwrap();
        assert_eq!(*fired.borrow(), vec![3]);

        let self_ptr = fixture.host.entity_address(1).unwrap();
        let other_ptr = fixture.host.entity_address(2).unwrap();
        let args = vec![
            GuestCallValue::Pointer(Some(self_ptr)),
            GuestCallValue::Pointer(Some(other_ptr)),
            GuestCallValue::Pointer(None),
        ];
        let ran = Rc::new(RefCell::new(false));
        let flag = ran.clone();
        fixture
            .combat
            .source_use(actor(1), &args, |_| {
                *flag.borrow_mut() = true;
                GuestCallResult::Void
            })
            .unwrap();
        assert!(*ran.borrow());
        assert_eq!(*fixture.routed.borrow(), vec!["use:1".to_string()]);
    }

    #[test]
    fn validation_deferred_and_touch_marshal() {
        let mut fixture = fixture();
        let mut bad = definition();
        bad.health = field(200);
        assert_eq!(
            NativeModCombat::new(
                bad,
                fixture.host.clone(),
                TestCalls {
                    host: fixture.host.clone(),
                    owners: HashMap::new(),
                    eligible: HashSet::new(),
                    owned: HashSet::new(),
                    synchronizes: Rc::new(RefCell::new(0)),
                },
                TestShared {
                    foreign: Rc::new(RefCell::new(Vec::new())),
                    routed: Rc::new(RefCell::new(Vec::new())),
                }
            )
            .map(|_| ()),
            Err(CombatError::CombatFieldRange)
        );

        write_i32(&fixture.host, 1, 20, 15);
        let request = damage_request(actor(1));
        fixture.combat.note_writes(1, 28, 1, Some(&request)).unwrap();
        let saved = fixture.combat.checkpoint_deferred().unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].blood, 15.0);
        fixture.combat.release(actor(1));
        assert!(fixture.combat.checkpoint_deferred().unwrap().is_empty());
        fixture.combat.restore_deferred(&saved).unwrap();
        assert_eq!(fixture.combat.checkpoint_deferred().unwrap().len(), 1);

        let touched = Rc::new(RefCell::new(0usize));
        let count = touched.clone();
        fixture.host.register_body(TOUCH_ENTRY, move |_, values| {
            *count.borrow_mut() = values.len();
            GuestCallResult::Void
        });
        let base = fixture.host.entity_address(1).unwrap();
        let code = GuestAddress::new(fixture.host.address_space(), TOUCH_ENTRY);
        fixture.host.with_memory(|memory| {
            let address = memory.offset(base, 40).unwrap();
            memory.write_pointer(address, Some(code)).unwrap();
        });
        fixture.combat.bind(1, actor(1)).unwrap();
        let contact = TouchContact {
            source: actor(1),
            other: actor(2),
            plane: Some(ContactPlane {
                normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
                distance: 4.0,
                plane_type: 2,
                signbits: 0,
            }),
            surface: Some(ContactSurface {
                name: "metal".to_string(),
                flags: 3,
                value: 5,
            }),
            trace: None,
        };
        fixture.combat.fire_touch(&contact).unwrap();
        assert_eq!(*touched.borrow(), 4);
        fixture.combat.close();
        assert_eq!(fixture.combat.watched_bytes(), None);
    }
}
