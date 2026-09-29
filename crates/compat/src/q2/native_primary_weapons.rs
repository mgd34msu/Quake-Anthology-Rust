//! Port of `src/compat/q2/native-primary-weapons.ts`.
//! Bridges primary weapon state: dispatcher tests, input reads and delay/damage factors.

use std::collections::HashMap;

use qa_core::math::Vec3;
use qa_guest::core::contracts::{
    ContentDigest, GuestAccess, GuestAddress, GuestAllocationOptions, GuestCallResult, GuestCallValue,
    GuestMapOptions, GuestPermissions, GuestRegister, ModuleIdentity, NativeAbi,
};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::error::GuestError;
use thiserror::Error;

use super::native_primary_reader::{
    NativeItemField, NativeItemTest, NativeRegion, NativeScalar, PointerExpectation, RecordKind,
    TestComparison,
};
use qa_core::identity::ProviderId;

/// Failure of a synthetic native host or primary weapon service operation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NativeHostError {
    /// Memory fault or contract violation with detail.
    #[error("native host fault: {0}")]
    Fault(String),
    /// Controlled exit: the input actor retired mid-dispatch.
    #[error("native input actor was removed")]
    Retired,
}

impl From<GuestError> for NativeHostError {
    fn from(error: GuestError) -> Self {
        Self::Fault(error.to_string())
    }
}

/// Result of a synthetic host operation.
pub type HostResult<T> = Result<T, NativeHostError>;

fn fault<T>(message: impl Into<String>) -> HostResult<T> {
    Err(NativeHostError::Fault(message.into()))
}

/// Generational id for one synthetic actor (entity slot plus generation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeActorId {
    /// Entity slot.
    pub slot: u32,
    /// Slot generation at spawn time.
    pub generation: u32,
}

/// One synthetic entity: guest record plus optional client record.
#[derive(Debug, Clone, Copy)]
struct HostEntity {
    address: GuestAddress,
    client: Option<GuestAddress>,
    live: bool,
}

/// Headless guest core: sparse memory, image base, entities and registers.
///
/// This is the synthetic stand-in for `MappedGuestMemory` plus the guest CPU
/// register file. There is no executable code; tests register per-address
/// originals on [`SyntheticHost`] and services invoke them explicitly.
pub struct HostCore {
    /// Sparse guest memory backing every record.
    pub memory: SparseGuestMemory,
    /// Synthetic image base; profile RVAs resolve against it.
    pub image: GuestAddress,
    /// Artifact digest this host emulates.
    pub digest: String,
    entities: Vec<HostEntity>,
    generations: Vec<u32>,
    registers: [u64; 16],
}

impl HostCore {
    /// Resolve an image-relative address.
    pub fn at(&self, rva: u32) -> HostResult<GuestAddress> {
        Ok(self.memory.offset(self.image, i64::from(rva))?)
    }

    /// Pointer width in bytes.
    #[must_use]
    pub fn pointer_bytes(&self) -> usize {
        self.memory.pointer_bytes()
    }

    /// Spawn an actor with fresh entity and client records.
    pub fn spawn_actor(&mut self, entity_bytes: usize, client_bytes: usize) -> HostResult<NativeActorId> {
        let mut options = GuestAllocationOptions::bytes(entity_bytes.max(1));
        options.alignment = 8;
        options.label = "synthetic entity".to_string();
        let entity = self.memory.allocate(&options)?;
        let mut options = GuestAllocationOptions::bytes(client_bytes.max(1));
        options.alignment = 8;
        options.label = "synthetic client".to_string();
        let client = self.memory.allocate(&options)?;
        let record = HostEntity {
            address: entity,
            client: Some(client),
            live: true,
        };
        if let Some((slot, current)) = self
            .entities
            .iter_mut()
            .enumerate()
            .find(|(_, entity)| !entity.live)
        {
            *current = record;
            let generation = self.generations[slot];
            return Ok(NativeActorId {
                slot: slot as u32,
                generation,
            });
        }
        let slot = self.entities.len() as u32;
        self.entities.push(record);
        self.generations.push(0);
        Ok(NativeActorId {
            slot,
            generation: 0,
        })
    }

    /// Retire an actor; its records stay mapped but no longer resolve.
    pub fn retire(&mut self, actor: NativeActorId) {
        if let Some(entity) = self.entities.get_mut(actor.slot as usize) {
            if self.generations[actor.slot as usize] == actor.generation {
                entity.live = false;
                self.generations[actor.slot as usize] = self.generations[actor.slot as usize].wrapping_add(1);
            }
        }
    }

    /// Whether an actor id still names a live entity.
    #[must_use]
    pub fn is_live(&self, actor: NativeActorId) -> bool {
        matches!(self.entities.get(actor.slot as usize), Some(entity) if entity.live)
            && self.generations.get(actor.slot as usize) == Some(&actor.generation)
    }

    /// Entity address for a live actor.
    pub fn entity_of(&self, actor: NativeActorId) -> HostResult<GuestAddress> {
        match self.entities.get(actor.slot as usize) {
            Some(entity)
                if entity.live && self.generations[actor.slot as usize] == actor.generation =>
            {
                Ok(entity.address)
            }
            _ => fault("native actor was retired"),
        }
    }

    /// Client address allocated for a live actor, if any.
    pub fn client_of(&self, actor: NativeActorId) -> HostResult<Option<GuestAddress>> {
        match self.entities.get(actor.slot as usize) {
            Some(entity)
                if entity.live && self.generations[actor.slot as usize] == actor.generation =>
            {
                Ok(entity.client)
            }
            _ => fault("native actor was retired"),
        }
    }

    /// Resolve the actor that owns an entity address.
    #[must_use]
    pub fn actor_for(&self, address: GuestAddress) -> Option<NativeActorId> {
        self.entities
            .iter()
            .enumerate()
            .find(|(_, entity)| entity.live && entity.address == address)
            .map(|(slot, _)| NativeActorId {
                slot: slot as u32,
                generation: self.generations[slot],
            })
    }

    /// Store the client link inside an entity record.
    pub fn set_client(
        &mut self,
        entity: GuestAddress,
        offset: u32,
        client: Option<GuestAddress>,
    ) -> HostResult<()> {
        let address = self.memory.offset(entity, i64::from(offset))?;
        Ok(self.memory.write_pointer(address, client)?)
    }

    /// Read a null-terminated classic string.
    pub fn read_c_string(&mut self, address: GuestAddress, maximum: usize) -> HostResult<String> {
        let mut bytes = Vec::new();
        for index in 0..maximum {
            let byte = self.memory.read_u8(self.memory.offset(address, index as i64)?)?;
            if byte == 0 {
                return String::from_utf8(bytes)
                    .map_err(|_| NativeHostError::Fault("classic string is not UTF-8".to_string()));
            }
            bytes.push(byte);
        }
        fault("unterminated classic string")
    }

    /// Write a null-terminated classic string into a fixed capacity.
    pub fn write_c_string(
        &mut self,
        address: GuestAddress,
        text: &str,
        capacity: usize,
    ) -> HostResult<()> {
        if text.len() + 1 > capacity || text.contains('\0') {
            return fault("classic string exceeds its guest allocation");
        }
        if !text.bytes().all(|byte| byte.is_ascii()) {
            return fault("classic strings require source byte characters");
        }
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0);
        Ok(self.memory.write(address, &bytes)?)
    }

    /// Allocate and store a null-terminated classic string.
    pub fn allocate_string(&mut self, text: &str) -> HostResult<GuestAddress> {
        let mut options = GuestAllocationOptions::bytes(text.len() + 1);
        options.alignment = 1;
        options.label = "synthetic string".to_string();
        let address = self.memory.allocate(&options)?;
        self.write_c_string(address, text, text.len() + 1)?;
        Ok(address)
    }

    /// Read one general-purpose register.
    #[must_use]
    pub fn register_read(&self, register: GuestRegister) -> u64 {
        self.registers[register.index()]
    }

    /// Write one general-purpose register.
    pub fn register_write(&mut self, register: GuestRegister, value: u64) {
        self.registers[register.index()] = value;
    }

    /// Read one scalar encoding as a number.
    pub fn read_scalar(&mut self, address: GuestAddress, encoding: NativeScalar) -> HostResult<f64> {
        match encoding {
            NativeScalar::Int8 => Ok(f64::from(self.memory.read_i8(address)?)),
            NativeScalar::Uint8 => Ok(f64::from(self.memory.read_u8(address)?)),
            NativeScalar::Int16 => Ok(f64::from(self.memory.read_i16(address)?)),
            NativeScalar::Uint16 => Ok(f64::from(self.memory.read_u16(address)?)),
            NativeScalar::Int32 => Ok(f64::from(self.memory.read_i32(address)?)),
            NativeScalar::Uint32 => Ok(f64::from(self.memory.read_u32(address)?)),
            NativeScalar::Float32 => Ok(f64::from(self.memory.read_f32(address)?)),
            NativeScalar::Float64 => Ok(self.memory.read_f64(address)?),
            NativeScalar::Int64 => {
                let value = self.memory.read_i64(address)?;
                if value.abs() > 9_007_199_254_740_991 {
                    return fault("native weapon field exceeds exact numeric range");
                }
                Ok(value as f64)
            }
            NativeScalar::Uint64 => {
                let value = self.memory.read_u64(address)?;
                if value > 9_007_199_254_740_991 {
                    return fault("native weapon field exceeds exact numeric range");
                }
                Ok(value as f64)
            }
        }
    }

    /// Write one scalar encoding from a number.
    pub fn write_scalar(
        &mut self,
        address: GuestAddress,
        encoding: NativeScalar,
        value: f64,
    ) -> HostResult<()> {
        if !value.is_finite() {
            return fault("native weapon field requires a finite value");
        }
        let integer = |low: f64, high: f64| {
            if value.fract() != 0.0 || value < low || value > high {
                return fault("native weapon field exceeds its encoding");
            }
            HostResult::Ok(())
        };
        match encoding {
            NativeScalar::Int8 => {
                integer(-128.0, 127.0)?;
                Ok(self.memory.write_i8(address, value as i8)?)
            }
            NativeScalar::Uint8 => {
                integer(0.0, 255.0)?;
                Ok(self.memory.write_u8(address, value as u8)?)
            }
            NativeScalar::Int16 => {
                integer(-32_768.0, 32_767.0)?;
                Ok(self.memory.write_i16(address, value as i16)?)
            }
            NativeScalar::Uint16 => {
                integer(0.0, 65_535.0)?;
                Ok(self.memory.write_u16(address, value as u16)?)
            }
            NativeScalar::Int32 => {
                integer(f64::from(i32::MIN), f64::from(i32::MAX))?;
                Ok(self.memory.write_i32(address, value as i32)?)
            }
            NativeScalar::Uint32 => {
                integer(0.0, f64::from(u32::MAX))?;
                Ok(self.memory.write_u32(address, value as u32)?)
            }
            NativeScalar::Int64 => {
                integer(-9_007_199_254_740_991.0, 9_007_199_254_740_991.0)?;
                Ok(self.memory.write_i64(address, value as i64)?)
            }
            NativeScalar::Uint64 => {
                integer(0.0, 9_007_199_254_740_991.0)?;
                Ok(self.memory.write_u64(address, value as u64)?)
            }
            NativeScalar::Float32 => Ok(self.memory.write_f32(address, value as f32)?),
            NativeScalar::Float64 => Ok(self.memory.write_f64(address, value)?),
        }
    }
}

/// Original routine emulated by a test: reads and writes `HostCore` directly.
pub type InvokeHandler =
    Box<dyn FnMut(&mut HostCore, &[GuestCallValue]) -> HostResult<GuestCallResult>>;

/// Recorded entry binding (name plus RVA) for wiring assertions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundEntry {
    /// Binding name.
    pub name: String,
    /// Entry RVA.
    pub rva: u32,
}

/// Synthetic native host: guest core plus registered originals and bindings.
pub struct SyntheticHost {
    /// Guest core shared by every primary service.
    pub core: HostCore,
    handlers: HashMap<u64, InvokeHandler>,
    bound_entries: Vec<BoundEntry>,
    bound_regions: Vec<NativeRegion>,
}

impl SyntheticHost {
    /// Build a host emulating `digest` with an `image_bytes` zeroed image.
    pub fn synthetic(
        digest: &str,
        pointer_bytes: usize,
        image_bytes: usize,
    ) -> HostResult<Self> {
        let (algorithm, value) = digest
            .split_once(':')
            .unwrap_or(("", digest));
        if algorithm.is_empty() || value.is_empty() {
            return fault("synthetic host requires an algorithm:hex digest");
        }
        let module = ModuleIdentity::new(
            ProviderId::new("q2", "synthetic-primary"),
            "synthetic.dll",
            ContentDigest::new(algorithm, value),
            "synthetic",
        );
        let mut memory = SparseGuestMemory::new(module, pointer_bytes, image_bytes as u64)?;
        let mut options = GuestMapOptions::new(0, image_bytes.max(1), GuestPermissions::ReadWriteExecute);
        options.label = "synthetic image".to_string();
        let image = memory.map(&options)?;
        Ok(Self {
            core: HostCore {
                memory,
                image,
                digest: digest.to_string(),
                entities: Vec::new(),
                generations: Vec::new(),
                registers: [0; 16],
            },
            handlers: HashMap::new(),
            bound_entries: Vec::new(),
            bound_regions: Vec::new(),
        })
    }

    /// Register the emulated original behind an absolute address.
    pub fn on_invoke(&mut self, address: GuestAddress, handler: InvokeHandler) {
        self.handlers.insert(address.offset, handler);
    }

    /// Register the emulated original behind an image RVA.
    pub fn on_rva(&mut self, rva: u32, handler: InvokeHandler) -> HostResult<()> {
        let address = self.core.at(rva)?;
        self.on_invoke(address, handler);
        Ok(())
    }

    /// Invoke a registered original.
    pub fn invoke(
        &mut self,
        address: GuestAddress,
        values: &[GuestCallValue],
    ) -> HostResult<GuestCallResult> {
        let mut handler = match self.handlers.remove(&address.offset) {
            Some(handler) => handler,
            None => return fault("synthetic host has no original at this address"),
        };
        let result = handler(&mut self.core, values);
        self.handlers.insert(address.offset, handler);
        result
    }

    /// Record an entry binding for wiring assertions.
    pub fn record_entry(&mut self, name: &str, rva: u32) {
        self.bound_entries.push(BoundEntry {
            name: name.to_string(),
            rva,
        });
    }

    /// Record an inline region for wiring assertions.
    pub fn record_region(&mut self, region: NativeRegion) {
        self.bound_regions.push(region);
    }

    /// Recorded entry bindings.
    #[must_use]
    pub fn bound_entries(&self) -> &[BoundEntry] {
        &self.bound_entries
    }

    /// Recorded inline regions.
    #[must_use]
    pub fn bound_regions(&self) -> &[NativeRegion] {
        &self.bound_regions
    }
}

/// Dispatcher declaration: entity argument position of the weapon call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponDispatcher {
    /// Dispatcher entry RVA.
    pub entry_rva: u32,
    /// Record carrying the actor argument.
    pub record: RecordKind,
    /// Actor argument index.
    pub argument: u32,
    /// Total argument count.
    pub arguments: u32,
}

/// One cleared input field inside a decision region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionField {
    /// Cleared field.
    pub field: NativeItemField,
    /// Bits cleared from the field.
    pub clear_mask: u32,
}

/// One weapon decision region with its cleared input fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponDecision {
    /// Region entry RVA.
    pub entry: u32,
    /// Region join RVA.
    pub join: u32,
    /// Cleared fields.
    pub fields: Vec<DecisionField>,
}

/// Spawn entry plus its acceptance tests.
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnGate {
    /// Spawn entry RVA.
    pub entry: u32,
    /// Acceptance tests.
    pub accepted: Vec<NativeItemTest>,
}

/// Source clock declaration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponTime {
    /// Clock address RVA.
    pub address: u32,
    /// Clock encoding.
    pub encoding: NativeScalar,
    /// Multiplier into milliseconds.
    pub milliseconds: f64,
}

/// Entity-side weapon fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponEntity {
    /// Client pointer offset within the entity.
    pub client: u32,
    /// Water level field.
    pub water_level: NativeItemField,
    /// View height field.
    pub view_height: NativeItemField,
    /// Max health field.
    pub max_health: NativeItemField,
}

/// Client-side weapon fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponClient {
    /// Client record length.
    pub byte_length: u32,
    /// View angles offset.
    pub view_angles: u32,
    /// Buttons field.
    pub buttons: NativeItemField,
    /// Latched buttons field.
    pub latched_buttons: NativeItemField,
}

/// Attack-animation entry plus skipped regions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttackAnimation {
    /// Entry RVA.
    pub entry: u32,
    /// Regions skipped while the animation runs.
    pub skip: Vec<NativeRegion>,
}

/// Animation frame fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponAnimation {
    /// Current frame.
    pub frame: NativeItemField,
    /// End frame.
    pub end: NativeItemField,
    /// Priority.
    pub priority: NativeItemField,
    /// Duck flag.
    pub duck: NativeItemField,
    /// Run flag.
    pub run: NativeItemField,
}

/// Equipment source context borrowed by the dispatcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EquipmentContext {
    /// Provider reference.
    pub provider: String,
    /// Optional item reference.
    pub item: Option<String>,
}

/// Projected animation field write.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectionWrite {
    /// Projected field.
    pub field: NativeItemField,
    /// Projected value.
    pub value: f64,
}

/// Firing-delay evaluation mode.
#[derive(Debug, Clone, PartialEq)]
pub enum DelayEvaluate {
    /// Flag-indexed factor table.
    SourceFlag {
        /// Factor per flag value.
        factors: Vec<f64>,
    },
    /// Source animation helper with projected fields.
    SourceAnimation {
        /// Helper entry RVA.
        entry: u32,
        /// Baseline milliseconds the helper result divides.
        baseline_milliseconds: f64,
        /// Projected fields.
        projection: Vec<ProjectionWrite>,
        /// Restored writes.
        writes: Vec<NativeItemField>,
    },
}

/// Firing-delay declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponDelay {
    /// Delay flag field.
    pub flag: NativeItemField,
    /// Delay capture region.
    pub region: NativeRegion,
    /// Evaluation mode.
    pub evaluate: DelayEvaluate,
}

/// Damage helper result width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageResult {
    /// Unsigned 8-bit result.
    Uint8,
    /// Signed 32-bit result.
    Int32,
}

/// Damage-factor declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum WeaponDamage {
    /// Helper result is the factor.
    SourceResult {
        /// Helper entry RVA.
        entry: u32,
        /// Result width.
        result: DamageResult,
    },
    /// Flag-indexed factor table.
    SourceFlag {
        /// Flag address RVA.
        address: u32,
        /// Flag encoding.
        encoding: NativeScalar,
        /// Factor per flag value.
        factors: Vec<f64>,
        /// Capture region.
        region: NativeRegion,
    },
}

/// Native primary weapon profile.
#[derive(Debug, Clone, PartialEq)]
pub struct NativePrimaryWeaponProfile {
    /// Artifact digest.
    pub digest: String,
    /// Executable ABI.
    pub abi: NativeAbi,
    /// Weapon dispatcher.
    pub dispatcher: WeaponDispatcher,
    /// Decision regions.
    pub decisions: Vec<WeaponDecision>,
    /// Spawn gate.
    pub spawn: SpawnGate,
    /// Availability tests.
    pub active: Vec<NativeItemTest>,
    /// Committed-input test groups.
    pub committed_input: Vec<Vec<NativeItemTest>>,
    /// Continuation test groups.
    pub continuations: Vec<Vec<NativeItemTest>>,
    /// Source clock.
    pub time: WeaponTime,
    /// Entity fields.
    pub entity: WeaponEntity,
    /// Client fields.
    pub client: WeaponClient,
    /// Attack animation.
    pub attack_animation: AttackAnimation,
    /// Animation fields.
    pub animation: WeaponAnimation,
    /// Equipment contexts.
    pub equipment_contexts: Vec<EquipmentContext>,
    /// Firing delay.
    pub delay: WeaponDelay,
    /// Damage factor.
    pub damage: WeaponDamage,
}

/// Weapon service hooks; the caller owns selection and spawn effects.
pub struct WeaponHooks {
    /// Whether the actor's weapon is selected.
    pub selected: Box<dyn FnMut(NativeActorId) -> bool>,
    /// Dispatcher completion notice.
    pub completed: Box<dyn FnMut(NativeActorId, bool)>,
    /// Spawn acceptance notice.
    pub spawned: Box<dyn FnMut(NativeActorId)>,
}

/// Borrowed client input snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponInput {
    /// Buttons value.
    pub buttons: f64,
    /// Latched buttons value.
    pub latched_buttons: f64,
    /// View angles.
    pub view_angles: Vec3,
    /// View height.
    pub view_height: f64,
    /// Water level.
    pub water_level: f64,
    /// Max health.
    pub max_health: f64,
}

/// Borrowed Q2 animation state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2Animation {
    /// Current frame.
    pub frame: f64,
    /// End frame.
    pub end_frame: f64,
    /// Priority.
    pub priority: f64,
    /// Ducking.
    pub duck: bool,
    /// Running.
    pub run: bool,
}

/// Wiring declared by [`NativePrimaryWeapons::bind`] for assertions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponWiring {
    /// Dispatcher entry RVA.
    pub dispatcher_entry: u32,
    /// Decision regions.
    pub decision_regions: Vec<NativeRegion>,
    /// Spawn entry RVA.
    pub spawn_entry: u32,
    /// Delay region.
    pub delay_region: NativeRegion,
    /// Damage capture region, when flag-based.
    pub damage_region: Option<NativeRegion>,
    /// Attack-animation skip regions.
    pub attack_skips: Vec<NativeRegion>,
}

/// Primary weapon service over a synthetic host.
pub struct NativePrimaryWeapons {
    profile: NativePrimaryWeaponProfile,
    hooks: WeaponHooks,
    delay_flags: HashMap<NativeActorId, f64>,
    damage_factors: HashMap<NativeActorId, f64>,
    bound: bool,
    closed: bool,
}

impl NativePrimaryWeapons {
    /// Build the service. Binding happens explicitly via [`Self::bind`].
    pub fn new(profile: NativePrimaryWeaponProfile, hooks: WeaponHooks) -> HostResult<Self> {
        if profile.dispatcher.argument >= profile.dispatcher.arguments {
            return fault("dispatcher actor argument is outside its call");
        }
        Ok(Self {
            profile,
            hooks,
            delay_flags: HashMap::new(),
            damage_factors: HashMap::new(),
            bound: false,
            closed: false,
        })
    }

    /// Borrow the profile.
    #[must_use]
    pub fn profile(&self) -> &NativePrimaryWeaponProfile {
        &self.profile
    }

    fn check_host(&self, host: &SyntheticHost) -> HostResult<()> {
        if self.closed {
            return fault("native primary weapon service is closed");
        }
        if host.core.digest != self.profile.digest {
            return fault("native primary weapon profile does not identify this artifact");
        }
        Ok(())
    }

    /// Record entry and region bindings on the host; headless equivalent of
    /// the donor constructor wiring.
    pub fn bind(&mut self, host: &mut SyntheticHost) -> HostResult<WeaponWiring> {
        self.check_host(host)?;
        if self.bound {
            return fault("native primary weapon service is already bound");
        }
        self.bound = true;
        let profile = &self.profile;
        host.record_entry("dispatcher", profile.dispatcher.entry_rva);
        host.record_entry("spawn", profile.spawn.entry);
        host.record_entry("attack-animation", profile.attack_animation.entry);
        for decision in &profile.decisions {
            host.record_region(NativeRegion {
                entry: decision.entry,
                join: decision.join,
            });
        }
        for skip in &profile.attack_animation.skip {
            host.record_region(*skip);
        }
        host.record_region(profile.delay.region);
        let damage_region = match &profile.damage {
            WeaponDamage::SourceResult { entry, .. } => {
                host.record_entry("damage", *entry);
                None
            }
            WeaponDamage::SourceFlag { region, .. } => {
                host.record_region(*region);
                Some(*region)
            }
        };
        if let DelayEvaluate::SourceAnimation { entry, .. } = &profile.delay.evaluate {
            host.record_entry("delay-evaluate", *entry);
        }
        Ok(WeaponWiring {
            dispatcher_entry: profile.dispatcher.entry_rva,
            decision_regions: profile
                .decisions
                .iter()
                .map(|decision| NativeRegion {
                    entry: decision.entry,
                    join: decision.join,
                })
                .collect(),
            spawn_entry: profile.spawn.entry,
            delay_region: profile.delay.region,
            damage_region,
            attack_skips: profile.attack_animation.skip.clone(),
        })
    }

    fn address(
        &self,
        host: &mut SyntheticHost,
        actor: NativeActorId,
        record: RecordKind,
        offset: u32,
    ) -> HostResult<GuestAddress> {
        match record {
            RecordKind::Image => host.core.at(offset),
            RecordKind::Entity => {
                let entity = host.core.entity_of(actor)?;
                Ok(host.core.memory.offset(entity, i64::from(offset))?)
            }
            RecordKind::Client => {
                let entity = host.core.entity_of(actor)?;
                let link = host
                    .core
                    .memory
                    .offset(entity, i64::from(self.profile.entity.client))?;
                let client = host.core.memory.read_pointer(link)?;
                let client = match client {
                    Some(client) => client,
                    None => return fault("native weapon owner has no source client"),
                };
                host.core.memory.check(
                    client,
                    self.profile.client.byte_length as usize,
                    GuestAccess::Read,
                )?;
                Ok(host.core.memory.offset(client, i64::from(offset))?)
            }
        }
    }

    /// Read one profile field for an actor.
    pub fn read(
        &self,
        host: &mut SyntheticHost,
        actor: NativeActorId,
        field: NativeItemField,
    ) -> HostResult<f64> {
        self.check_host(host)?;
        let address = self.address(host, actor, field.record, field.offset)?;
        host.core.read_scalar(address, field.encoding)
    }

    /// Write one profile field for an actor.
    pub fn write(
        &self,
        host: &mut SyntheticHost,
        actor: NativeActorId,
        field: NativeItemField,
        value: f64,
    ) -> HostResult<()> {
        self.check_host(host)?;
        let address = self.address(host, actor, field.record, field.offset)?;
        host.core.write_scalar(address, field.encoding, value)
    }

    /// Whether an actor id still names its live entity.
    pub fn current(&self, host: &SyntheticHost, actor: NativeActorId) -> bool {
        !self.closed && host.core.is_live(actor)
    }

    /// Borrowed client input snapshot.
    pub fn input(&self, host: &mut SyntheticHost, actor: NativeActorId) -> HostResult<WeaponInput> {
        self.check_host(host)?;
        let client = &self.profile.client;
        let angles = self.address(host, actor, RecordKind::Client, client.view_angles)?;
        let x = host.core.memory.read_f32(angles)?;
        let y = host
            .core
            .memory
            .read_f32(host.core.memory.offset(angles, 4)?)?;
        let z = host
            .core
            .memory
            .read_f32(host.core.memory.offset(angles, 8)?)?;
        Ok(WeaponInput {
            buttons: self.read(host, actor, client.buttons)?,
            latched_buttons: self.read(host, actor, client.latched_buttons)?,
            view_angles: Vec3 { x, y, z },
            view_height: self.read(host, actor, self.profile.entity.view_height)?,
            water_level: self.read(host, actor, self.profile.entity.water_level)?,
            max_health: self.read(host, actor, self.profile.entity.max_health)?,
        })
    }

    /// Borrowed animation state.
    pub fn animation(&self, host: &mut SyntheticHost, actor: NativeActorId) -> HostResult<Q2Animation> {
        self.check_host(host)?;
        let fields = &self.profile.animation;
        Ok(Q2Animation {
            frame: self.read(host, actor, fields.frame)?,
            end_frame: self.read(host, actor, fields.end)?,
            priority: self.read(host, actor, fields.priority)?,
            duck: self.read(host, actor, fields.duck)? != 0.0,
            run: self.read(host, actor, fields.run)? != 0.0,
        })
    }

    /// Evaluate one acceptance test.
    pub fn matches(
        &self,
        host: &mut SyntheticHost,
        actor: NativeActorId,
        test: &NativeItemTest,
    ) -> HostResult<bool> {
        self.check_host(host)?;
        match test {
            NativeItemTest::Pointer {
                record,
                offset,
                value,
            } => {
                let address = self.address(host, actor, *record, *offset)?;
                let pointer = host.core.memory.read_pointer(address)?;
                let mut expected = match value {
                    None => None,
                    Some(PointerExpectation { rva, .. }) => Some(host.core.at(*rva)?),
                };
                if let Some(PointerExpectation { indirections, .. }) = value {
                    for displacement in indirections {
                        expected = match expected {
                            None => None,
                            Some(address) => host
                                .core
                                .memory
                                .read_pointer(host.core.memory.offset(address, i64::from(*displacement))?)?,
                        };
                    }
                }
                Ok(pointer == expected)
            }
            NativeItemTest::Scalar {
                field,
                mask,
                comparison,
                value,
            } => {
                let raw = self.read(host, actor, *field)?;
                let masked = match mask {
                    None => raw,
                    Some(mask) => ((raw as i64) & i64::from(*mask)) as f64,
                };
                Ok(match comparison {
                    TestComparison::Equals => masked == *value,
                    TestComparison::AtMost => masked <= *value,
                })
            }
        }
    }

    /// Whether the weapon is available for an actor.
    pub fn available(&self, host: &mut SyntheticHost, actor: NativeActorId) -> HostResult<bool> {
        if !self.current(host, actor) {
            return Ok(false);
        }
        for test in &self.profile.active {
            if !self.matches(host, actor, test)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Whether a continuation group currently matches.
    pub fn continuing(&self, host: &mut SyntheticHost, actor: NativeActorId) -> HostResult<bool> {
        if !self.current(host, actor) {
            return Ok(false);
        }
        for group in &self.profile.continuations {
            let mut every = true;
            for test in group {
                if !self.matches(host, actor, test)? {
                    every = false;
                    break;
                }
            }
            if every {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether committed input currently matches.
    pub fn committed_input(&self, host: &mut SyntheticHost, actor: NativeActorId) -> HostResult<bool> {
        if !self.current(host, actor) {
            return Ok(false);
        }
        for group in &self.profile.committed_input {
            let mut every = true;
            for test in group {
                if !self.matches(host, actor, test)? {
                    every = false;
                    break;
                }
            }
            if every {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Source clock in milliseconds.
    pub fn time_milliseconds(&self, host: &mut SyntheticHost) -> HostResult<f64> {
        self.check_host(host)?;
        let address = host.core.at(self.profile.time.address)?;
        Ok(host.core.read_scalar(address, self.profile.time.encoding)? * self.profile.time.milliseconds)
    }

    /// Headless equivalent of the spawn entry wrapper: report acceptance and
    /// fire the spawn hook for accepted actors.
    pub fn note_spawn(&mut self, host: &mut SyntheticHost, actor: NativeActorId) -> HostResult<bool> {
        self.check_host(host)?;
        if !self.current(host, actor) {
            return Ok(false);
        }
        for test in &self.profile.spawn.accepted {
            if !self.matches(host, actor, test)? {
                return Ok(false);
            }
        }
        (self.hooks.spawned)(actor);
        Ok(true)
    }

    /// Headless equivalent of the dispatcher selection query.
    pub fn dispatch_selected(&mut self, host: &SyntheticHost, actor: NativeActorId) -> bool {
        self.current(host, actor) && (self.hooks.selected)(actor)
    }

    /// Headless equivalent of dispatcher completion.
    pub fn note_dispatch_completed(&mut self, actor: NativeActorId, reached_decision: bool) {
        (self.hooks.completed)(actor, reached_decision);
    }

    /// Headless equivalent of the delay capture region: sample the flag.
    pub fn record_delay_sample(
        &mut self,
        host: &mut SyntheticHost,
        actor: NativeActorId,
    ) -> HostResult<()> {
        self.check_host(host)?;
        if !self.current(host, actor) {
            return fault("native weapon owner was retired");
        }
        let flag = self.read(host, actor, self.profile.delay.flag)?;
        self.delay_flags.insert(actor, flag);
        Ok(())
    }

    /// Headless equivalent of the damage capture region: sample the factor.
    pub fn record_damage_sample(
        &mut self,
        host: &mut SyntheticHost,
        actor: NativeActorId,
    ) -> HostResult<()> {
        self.check_host(host)?;
        if !self.current(host, actor) {
            return fault("native weapon owner was retired");
        }
        let (address, encoding, factors) = match &self.profile.damage {
            WeaponDamage::SourceResult { .. } => {
                return fault("result damage has no capture region");
            }
            WeaponDamage::SourceFlag {
                address,
                encoding,
                factors,
                ..
            } => (*address, *encoding, factors.clone()),
        };
        let value = host.core.read_scalar(host.core.at(address)?, encoding)?;
        let factor = factors
            .get(value as usize)
            .copied()
            .unwrap_or_else(|| f64::NAN);
        if factor.is_nan() {
            return fault("original weapon damage flag has no declared source factor");
        }
        self.damage_factors.insert(actor, factor);
        Ok(())
    }

    /// Damage factor for an actor: sampled factor or helper result.
    pub fn damage_factor(
        &mut self,
        host: &mut SyntheticHost,
        actor: NativeActorId,
    ) -> HostResult<f64> {
        self.check_host(host)?;
        match &self.profile.damage {
            WeaponDamage::SourceFlag { .. } => {
                host.core.entity_of(actor)?;
                match self.damage_factors.get(&actor) {
                    Some(factor) => Ok(*factor),
                    None => fault("original weapon damage modifier has not run for this actor"),
                }
            }
            WeaponDamage::SourceResult { entry, .. } => {
                let entity = host.core.entity_of(actor)?;
                let address = host.core.at(*entry)?;
                let result = host.invoke(address, &[GuestCallValue::Pointer(Some(entity))])?;
                match result {
                    GuestCallResult::Value(GuestCallValue::Int32(value)) => Ok(f64::from(value)),
                    GuestCallResult::Value(GuestCallValue::Uint32(value)) => Ok(f64::from(value)),
                    _ => fault("original weapon damage helper returned the wrong ABI type"),
                }
            }
        }
    }

    /// Run the attack animation with its skip regions bound. Returns false
    /// when the weapon is unavailable, mirroring the donor early return.
    pub fn attack_animation(
        &self,
        host: &mut SyntheticHost,
        actor: NativeActorId,
    ) -> HostResult<bool> {
        self.check_host(host)?;
        if !self.available(host, actor)? {
            return Ok(false);
        }
        for skip in &self.profile.attack_animation.skip {
            host.record_region(*skip);
        }
        let entity = host.core.entity_of(actor)?;
        let address = host.core.at(self.profile.attack_animation.entry)?;
        let values = [
            GuestCallValue::Pointer(Some(entity)),
            GuestCallValue::Int32(0),
            GuestCallValue::Int32(0),
            GuestCallValue::Int32(0),
            GuestCallValue::Int32(0),
            GuestCallValue::Pointer(None),
            GuestCallValue::Pointer(None),
            GuestCallValue::Pointer(None),
        ];
        host.invoke(address, &values)?;
        Ok(true)
    }

    /// Scale firing time by the sampled delay factor.
    pub fn weapon_delay(
        &self,
        host: &mut SyntheticHost,
        actor: NativeActorId,
        milliseconds: f64,
    ) -> HostResult<f64> {
        self.check_host(host)?;
        let flag = match self.delay_flags.get(&actor) {
            Some(flag) => *flag,
            None => return fault("original firing modifier has not run for this actor"),
        };
        match &self.profile.delay.evaluate {
            DelayEvaluate::SourceFlag { factors } => {
                let factor = factors.get(flag as usize).copied().unwrap_or(f64::NAN);
                if factor.is_nan() {
                    return fault("original firing flag has no declared source factor");
                }
                Ok(milliseconds * factor)
            }
            DelayEvaluate::SourceAnimation {
                entry,
                baseline_milliseconds,
                projection,
                writes,
            } => {
                let mut fields: Vec<NativeItemField> =
                    projection.iter().map(|write| write.field).collect();
                fields.extend(writes.iter().copied());
                fields.push(self.profile.delay.flag);
                let mut saved = Vec::with_capacity(fields.len());
                for field in &fields {
                    saved.push((*field, self.read(host, actor, *field)?));
                }
                let mut options = GuestAllocationOptions::bytes(8);
                options.alignment = 8;
                options.label = "selected source weapon delay".to_string();
                let result_storage = host.core.memory.allocate(&options)?;
                let outcome = (|| {
                    for write in projection {
                        self.write(host, actor, write.field, write.value)?;
                    }
                    self.write(host, actor, self.profile.delay.flag, flag)?;
                    let entity = host.core.entity_of(actor)?;
                    let address = host.core.at(*entry)?;
                    host.invoke(
                        address,
                        &[
                            GuestCallValue::Pointer(Some(result_storage)),
                            GuestCallValue::Pointer(Some(entity)),
                        ],
                    )?;
                    Ok::<f64, NativeHostError>(
                        milliseconds * (host.core.memory.read_i64(result_storage)? as f64)
                            / *baseline_milliseconds,
                    )
                })();
                for (field, value) in saved {
                    if self.current(host, actor) {
                        self.write(host, actor, field, value)?;
                    }
                }
                host.core.memory.unmap(result_storage, 8)?;
                outcome
            }
        }
    }

    /// Forget sampled state for an actor.
    pub fn release(&mut self, actor: NativeActorId) {
        self.damage_factors.remove(&actor);
        self.delay_flags.remove(&actor);
    }

    /// Clear sampled state and close the service.
    pub fn close(&mut self) {
        self.closed = true;
        self.damage_factors.clear();
        self.delay_flags.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_primary_reader::CLASSIC_DIGEST;
    use super::*;

    fn scalar_field(record: RecordKind, offset: u32, encoding: NativeScalar) -> NativeItemField {
        NativeItemField {
            record,
            offset,
            encoding,
        }
    }

    fn fixture_profile() -> NativePrimaryWeaponProfile {
        NativePrimaryWeaponProfile {
            digest: CLASSIC_DIGEST.to_string(),
            abi: NativeAbi::WindowsI386,
            dispatcher: WeaponDispatcher {
                entry_rva: 0x100,
                record: RecordKind::Entity,
                argument: 0,
                arguments: 1,
            },
            decisions: vec![WeaponDecision {
                entry: 0x110,
                join: 0x120,
                fields: vec![DecisionField {
                    field: scalar_field(RecordKind::Client, 8, NativeScalar::Int32),
                    clear_mask: 1,
                }],
            }],
            spawn: SpawnGate {
                entry: 0x130,
                accepted: vec![NativeItemTest::Scalar {
                    field: scalar_field(RecordKind::Client, 12, NativeScalar::Int32),
                    mask: None,
                    comparison: TestComparison::Equals,
                    value: 0.0,
                }],
            },
            active: vec![NativeItemTest::Scalar {
                field: scalar_field(RecordKind::Entity, 16, NativeScalar::Int32),
                mask: None,
                comparison: TestComparison::Equals,
                value: 0.0,
            }],
            committed_input: vec![],
            continuations: vec![vec![NativeItemTest::Scalar {
                field: scalar_field(RecordKind::Client, 12, NativeScalar::Int32),
                mask: None,
                comparison: TestComparison::Equals,
                value: 3.0,
            }]],
            time: WeaponTime {
                address: 0x200,
                encoding: NativeScalar::Float32,
                milliseconds: 1000.0,
            },
            entity: WeaponEntity {
                client: 84,
                water_level: scalar_field(RecordKind::Entity, 20, NativeScalar::Int32),
                view_height: scalar_field(RecordKind::Entity, 24, NativeScalar::Int32),
                max_health: scalar_field(RecordKind::Entity, 28, NativeScalar::Int32),
            },
            client: WeaponClient {
                byte_length: 256,
                view_angles: 32,
                buttons: scalar_field(RecordKind::Client, 8, NativeScalar::Int32),
                latched_buttons: scalar_field(RecordKind::Client, 44, NativeScalar::Int32),
            },
            attack_animation: AttackAnimation {
                entry: 0x300,
                skip: vec![NativeRegion { entry: 0x310, join: 0x320 }],
            },
            animation: WeaponAnimation {
                frame: scalar_field(RecordKind::Entity, 56, NativeScalar::Int32),
                end: scalar_field(RecordKind::Client, 48, NativeScalar::Int32),
                priority: scalar_field(RecordKind::Client, 52, NativeScalar::Int32),
                duck: scalar_field(RecordKind::Client, 56, NativeScalar::Int32),
                run: scalar_field(RecordKind::Client, 60, NativeScalar::Int32),
            },
            equipment_contexts: vec![],
            delay: WeaponDelay {
                flag: scalar_field(RecordKind::Image, 0x400, NativeScalar::Int32),
                region: NativeRegion {
                    entry: 0x410,
                    join: 0x420,
                },
                evaluate: DelayEvaluate::SourceFlag {
                    factors: vec![1.0, 0.5],
                },
            },
            damage: WeaponDamage::SourceFlag {
                address: 0x404,
                encoding: NativeScalar::Int32,
                factors: vec![1.0, 4.0],
                region: NativeRegion {
                    entry: 0x430,
                    join: 0x440,
                },
            },
        }
    }

    fn fixture_hooks() -> WeaponHooks {
        WeaponHooks {
            selected: Box::new(|_| true),
            completed: Box::new(|_, _| {}),
            spawned: Box::new(|_| {}),
        }
    }

    fn fixture_host() -> SyntheticHost {
        SyntheticHost::synthetic(CLASSIC_DIGEST, 4, 0x1000).expect("host")
    }

    fn spawn_linked(host: &mut SyntheticHost) -> NativeActorId {
        let actor = host.core.spawn_actor(896, 256).expect("actor");
        let entity = host.core.entity_of(actor).expect("entity");
        let client = host.core.client_of(actor).expect("client");
        host.core.set_client(entity, 84, client).expect("link");
        actor
    }

    #[test]
    fn binds_wiring_and_reads_input() {
        let mut host = fixture_host();
        let mut service =
            NativePrimaryWeapons::new(fixture_profile(), fixture_hooks()).expect("service");
        let wiring = service.bind(&mut host).expect("bind");
        assert_eq!(wiring.dispatcher_entry, 0x100);
        assert_eq!(wiring.decision_regions.len(), 1);
        assert_eq!(wiring.damage_region.is_some(), true);
        assert!(host.bound_entries().iter().any(|entry| entry.rva == 0x300));
        let actor = spawn_linked(&mut host);
        service
            .write(
                &mut host,
                actor,
                scalar_field(RecordKind::Client, 8, NativeScalar::Int32),
                7.0,
            )
            .expect("write");
        service
            .write(
                &mut host,
                actor,
                scalar_field(RecordKind::Entity, 28, NativeScalar::Int32),
                100.0,
            )
            .expect("write");
        let input = service.input(&mut host, actor).expect("input");
        assert_eq!(input.buttons, 7.0);
        assert_eq!(input.max_health, 100.0);
        assert_eq!(input.view_angles, Vec3 { x: 0.0, y: 0.0, z: 0.0 });
        assert!(service.available(&mut host, actor).expect("available"));
        assert!(!service.continuing(&mut host, actor).expect("continuing"));
    }

    #[test]
    fn samples_delay_and_damage_factors() {
        let mut host = fixture_host();
        let mut service =
            NativePrimaryWeapons::new(fixture_profile(), fixture_hooks()).expect("service");
        service.bind(&mut host).expect("bind");
        let actor = spawn_linked(&mut host);
        let flag = host.core.at(0x400).expect("flag");
        host.core.memory.write_i32(flag, 1).expect("write flag");
        let damage = host.core.at(0x404).expect("damage");
        host.core.memory.write_i32(damage, 1).expect("write damage");
        service
            .record_delay_sample(&mut host, actor)
            .expect("delay");
        service
            .record_damage_sample(&mut host, actor)
            .expect("damage");
        assert_eq!(
            service.weapon_delay(&mut host, actor, 100.0).expect("delay"),
            50.0
        );
        assert_eq!(service.damage_factor(&mut host, actor).expect("factor"), 4.0);
        service.release(actor);
        assert!(service.damage_factor(&mut host, actor).is_err());
    }

    #[test]
    fn matches_pointer_tests_through_indirections() {
        let mut host = fixture_host();
        let service =
            NativePrimaryWeapons::new(fixture_profile(), fixture_hooks()).expect("service");
        let actor = spawn_linked(&mut host);
        let target = host.core.at(0x500).expect("target");
        let entity = host.core.entity_of(actor).expect("entity");
        host.core
            .memory
            .write_pointer(
                host.core.memory.offset(entity, 64).expect("slot"),
                Some(target),
            )
            .expect("write pointer");
        let direct = NativeItemTest::Pointer {
            record: RecordKind::Entity,
            offset: 64,
            value: Some(PointerExpectation {
                rva: 0x500,
                indirections: vec![],
            }),
        };
        assert!(service.matches(&mut host, actor, &direct).expect("match"));
        let mismatch = NativeItemTest::Pointer {
            record: RecordKind::Entity,
            offset: 64,
            value: None,
        };
        assert!(!service.matches(&mut host, actor, &mismatch).expect("match"));
    }

    #[test]
    fn attack_animation_invokes_with_skip_regions() {
        use std::sync::{Arc, Mutex};
        let mut host = fixture_host();
        let service =
            NativePrimaryWeapons::new(fixture_profile(), fixture_hooks()).expect("service");
        let actor = spawn_linked(&mut host);
        let seen: Arc<Mutex<Vec<GuestCallValue>>> = Arc::new(Mutex::new(Vec::new()));
        let capture = seen.clone();
        host.on_rva(
            0x300,
            Box::new(move |_, values| {
                *capture.lock().expect("lock") = values.to_vec();
                Ok(GuestCallResult::Void)
            }),
        )
        .expect("handler");
        assert!(service.attack_animation(&mut host, actor).expect("attack"));
        let values = seen.lock().expect("lock").clone();
        assert_eq!(values.len(), 8);
        assert!(matches!(values[0], GuestCallValue::Pointer(Some(_))));
        assert!(host.bound_regions().contains(&NativeRegion {
            entry: 0x310,
            join: 0x320,
        }));
    }

    #[test]
    fn rejects_foreign_artifacts() {
        let mut host =
            SyntheticHost::synthetic(super::super::native_primary_reader::RETAIL_DIGEST, 8, 0x1000)
                .expect("host");
        let mut service =
            NativePrimaryWeapons::new(fixture_profile(), fixture_hooks()).expect("service");
        assert!(service.bind(&mut host).is_err());
    }
}
