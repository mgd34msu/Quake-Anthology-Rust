//! Classic Quake II guest import services.
//!
//! Port of donor `src/app/bootstrap/simulation/classic-guest-services.ts`
//! (`ClassicGuestServices`, `ClassicGuestServicesOptions`,
//! `ClassicGuestMapServices`, `ClassicGuestMessage`, `ClassicGuestAudience`).
//!
//! The donor hands the services object to the DLL-running host, which calls
//! the import closures back. The Rust [`ClassicQ2GuestHost`] owns its memory
//! and answers imports from its scripted engine instead, so the import
//! implementations live here as methods taking the host explicitly, and the
//! world drives them. Live engine projections the Rust tables cannot express
//! (`bodies.bind` write-through, `combat.bind`, link-state restore) are
//! snapshot-synced or reported; see the method notes.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_compat::q2::classic::combat_binding::ClassicCombatBindings;
use qa_compat::q2::classic::combat_profile::classic_combat_profile;
use qa_compat::q2::classic::host::{ClassicQ2GuestHost, ClassicWorldLink};
use qa_compat::q2::classic::inventory::{ClassicInventoryBinding, ClassicSourceInventory};
use qa_compat::q2::classic::layout::{ClassicQ2Error, ClassicResult};
use qa_compat::q2::classic::pmove::EquipmentMovement;
use qa_compat::q2::classic::records::{read_classic_string, read_classic_vector, RawEntityView};
use qa_compat::q2::classic::world_profile::ClassicPrimaryWorldProfile;
use qa_compat::q2::native_pickups::NativePickupProfile;
use qa_content::contract::{ItemId, OriginalPickupAdmission};
use qa_content::q2::foundation::host::{Q2FoundationHost, Q2ModelEvent, Q2PresentationEvent};
use qa_content::q2::support::contracts::BodyState as EngineBodyState;
use qa_core::cvar::CvarRegistry;
use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::math::{Bounds, Vec3};
use qa_core::numeric::{float_to_wrapped_i32, NumericOps};
use qa_core::time::SourceTime;
use qa_guest::core::contracts::{GuestAddress, GuestCallValue};
use qa_guest::core::memory::SparseGuestMemory;
use qa_net::msg::{MsgError, MsgWriter};
use qa_net::q2::write_dir;
use qa_net::q2_adapters::{Q2EntityState, Q2MovementState, Q2PlayerState, Q2PlayerView, Q2Vec3, Q2Vec4};
use qa_world::body::BodyState as WorldBodyState;
use qa_world::spatial::{ActorCollision, CollisionFamily, CollisionRole, CollisionShape, QueryRole};

use super::source_hosts::ActorHostScene;

/// Multicast scope for a guest message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MulticastScope {
    /// Every client.
    All,
    /// Potentially hearable set.
    Phs,
    /// Potentially visible set.
    Pvs,
}

/// Audience of a guest message.
#[derive(Debug, Clone, PartialEq)]
pub enum ClassicGuestAudience {
    /// One client slot.
    Unicast {
        /// Client slot.
        slot: u32,
    },
    /// Spatial or global multicast.
    Multicast {
        /// Multicast origin.
        origin: Vec3,
        /// Multicast scope.
        scope: MulticastScope,
    },
}

/// Queued guest message.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassicGuestMessage {
    /// Message audience.
    pub audience: ClassicGuestAudience,
    /// Reliable delivery.
    pub reliable: bool,
    /// Message bytes.
    pub bytes: Vec<u8>,
}

/// Current guest command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestCommandLine {
    /// Command arguments.
    pub arguments: Vec<String>,
    /// Raw argument text.
    pub args: String,
}

/// Mirror of `Omit<AttackProvenance, "attacker" | "inflictor" | "cause">`
/// from donor `src/compat/q2/classic/host.ts` (canonical home:
/// `qa_compat::q2::classic::host`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct AttackProvenanceTail {
    /// Source sequence number.
    pub sequence: u64,
    /// Source time of the attack.
    pub time: SourceTime,
    /// Originating projectile, when routed through one.
    pub originating_projectile: Option<ActorId>,
    /// Attacking weapon item.
    pub weapon: Option<ItemId>,
    /// Weapon provider.
    pub weapon_provider: ProviderId,
    /// This source already applied its damage modifier.
    pub damage_powerup_owner: Option<ProviderId>,
    /// Combat provider.
    pub combat_provider: ProviderId,
    /// Inventory provider.
    pub inventory_provider: ProviderId,
    /// Movement provider.
    pub movement_provider: ProviderId,
}

/// Native damage provenance callback.
pub type ClassicDamageProvenance = Rc<dyn Fn(&ActorId, &ActorId, &ActorId) -> AttackProvenanceTail>;

/// Opaque native pickup admission. The donor threads
/// `OriginalPickupAdmission` (canonical home: `qa_content::contract`)
/// through to the host; its generic `run_source` method is not object-safe,
/// and no Rust host consumes it yet, so the services retain the typed value
/// opaquely and report presence to the host binding.
#[derive(Clone)]
pub struct GuestPickupAdmission {
    inner: Rc<dyn std::any::Any>,
}

impl GuestPickupAdmission {
    /// Retain a typed admission opaquely.
    #[must_use]
    pub fn new<T: OriginalPickupAdmission + 'static>(admission: T) -> Self {
        Self {
            inner: Rc::new(admission),
        }
    }

    /// Recover the retained admission.
    #[must_use]
    pub fn get<T: OriginalPickupAdmission + 'static>(&self) -> Option<&T> {
        self.inner.downcast_ref::<T>()
    }
}

impl std::fmt::Debug for GuestPickupAdmission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("GuestPickupAdmission").finish_non_exhaustive()
    }
}

/// Entity admission callback.
pub type GuestAdmitFn = Box<dyn FnMut(&RawEntityView, &OwnedActor)>;
/// Body collision callback.
pub type GuestCollisionFn = Box<dyn FnMut(&OwnedActor, &ActorCollision)>;
/// Guest print sink.
pub type GuestPrintFn = Box<dyn FnMut(&str)>;
/// Current command line reader.
pub type GuestCommandFn = Rc<dyn Fn() -> GuestCommandLine>;
/// Console command sink.
pub type GuestAddCommandFn = Box<dyn FnMut(&str)>;
/// Debug graph sink.
pub type GuestDebugGraphFn = Box<dyn FnMut(f64, i32)>;
/// Client acceptance probe.
pub type GuestAcceptsClientFn = Rc<dyn Fn(u32) -> bool>;
/// Player velocity writer.
pub type PlayerVelocityWrite = Box<dyn FnMut(&OwnedActor, Vec3)>;
/// Player velocity reader.
pub type PlayerVelocityRead = Box<dyn FnMut(&ActorId) -> Option<Vec3>>;

/// Classic guest services options.
pub struct ClassicGuestServicesOptions {
    /// Declared primary world profile.
    pub primary_world: Option<ClassicPrimaryWorldProfile>,
    /// Declared native pickup profile.
    pub pickup_profile: Option<NativePickupProfile>,
    /// Native pickup admission.
    pub pickups: Option<GuestPickupAdmission>,
    /// Native damage provenance.
    pub damage_provenance: Option<ClassicDamageProvenance>,
    /// Engine authorities.
    pub engine: Box<dyn Q2FoundationHost>,
    /// Shared scene queries.
    pub scene: Rc<dyn ActorHostScene>,
    /// Console variables.
    pub cvars: CvarRegistry,
    /// Numeric operations.
    pub numeric: NumericOps,
    /// Map path.
    pub map_path: String,
    /// Maximum clients.
    pub max_clients: u32,
    /// Client acceptance probe.
    pub accepts_client: Option<GuestAcceptsClientFn>,
    /// Entity admission callback.
    pub admit: GuestAdmitFn,
    /// Body collision callback.
    pub collision: GuestCollisionFn,
    /// Guest print sink.
    pub print: GuestPrintFn,
    /// Current command line reader.
    pub command: GuestCommandFn,
    /// Console command sink.
    pub add_command: GuestAddCommandFn,
    /// Debug graph sink.
    pub debug_graph: GuestDebugGraphFn,
}

/// World-rebindable subset of the services options.
pub struct ClassicGuestMapServices {
    /// Engine authorities.
    pub engine: Box<dyn Q2FoundationHost>,
    /// Shared scene queries.
    pub scene: Rc<dyn ActorHostScene>,
    /// Map path.
    pub map_path: String,
    /// Entity admission callback.
    pub admit: GuestAdmitFn,
    /// Body collision callback.
    pub collision: GuestCollisionFn,
    /// Guest print sink.
    pub print: GuestPrintFn,
    /// Current command line reader.
    pub command: GuestCommandFn,
    /// Console command sink.
    pub add_command: GuestAddCommandFn,
    /// Debug graph sink.
    pub debug_graph: GuestDebugGraphFn,
    /// Native damage provenance.
    pub damage_provenance: Option<ClassicDamageProvenance>,
    /// Native pickup admission.
    pub pickups: Option<GuestPickupAdmission>,
}

/// Configstring resource kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    /// Model resources.
    Model,
    /// Sound resources.
    Sound,
    /// Image resources.
    Image,
}

impl ResourceKind {
    fn base(self) -> i32 {
        match self {
            Self::Model => 32,
            Self::Sound => 288,
            Self::Image => 544,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Sound => "sound",
            Self::Image => "image",
        }
    }
}

#[derive(Debug, Clone)]
struct NameSlots {
    slots: HashSet<usize>,
    first: usize,
}

/// Mirror of `ResourceNameIndex` from donor
/// `src/core/resource-name-index.ts` (canonical home:
/// `qa_core::resource_name_index`); unify post-merge.
#[derive(Debug, Clone)]
pub struct ResourceNameIndex {
    values: Vec<String>,
    reserved: HashSet<usize>,
    names: HashMap<String, NameSlots>,
    free: usize,
}

impl ResourceNameIndex {
    /// Create an index with `capacity` slots and reserved indexes.
    pub fn new(capacity: usize, reserved: &[usize]) -> Self {
        if capacity < 1 {
            panic!("Invalid resource capacity");
        }
        let mut index = Self {
            values: vec![String::new(); capacity],
            reserved: reserved.iter().copied().collect(),
            names: HashMap::new(),
            free: 1,
        };
        index.advance();
        index
    }

    /// Clear every slot.
    pub fn clear(&mut self) {
        self.values.fill(String::new());
        self.names.clear();
        self.free = 1;
        self.advance();
    }

    /// Record `value` at `index`, ignoring out-of-range and reserved slots.
    pub fn set(&mut self, index: i32, value: &str) {
        if index <= 0 || index as usize >= self.values.len() || self.reserved.contains(&(index as usize)) {
            return;
        }
        let slot = index as usize;
        let previous = std::mem::replace(&mut self.values[slot], value.to_string());
        if previous == value {
            return;
        }
        if !previous.is_empty() {
            let remove = if let Some(entry) = self.names.get_mut(&previous) {
                entry.slots.remove(&slot);
                if entry.slots.is_empty() {
                    true
                } else {
                    if entry.first == slot {
                        entry.first = *entry.slots.iter().min().unwrap_or(&self.values.len());
                    }
                    false
                }
            } else {
                false
            };
            if remove {
                self.names.remove(&previous);
            }
        }
        if value.is_empty() {
            self.free = self.free.min(slot);
        } else {
            match self.names.get_mut(value) {
                None => {
                    self.names.insert(
                        value.to_string(),
                        NameSlots {
                            slots: HashSet::from([slot]),
                            first: slot,
                        },
                    );
                }
                Some(entry) => {
                    entry.slots.insert(slot);
                    entry.first = entry.first.min(slot);
                }
            }
            if slot == self.free {
                self.advance();
            }
        }
    }

    /// Find the slot for `name`, or the next free slot.
    pub fn find(&self, name: &str) -> Option<i32> {
        if name.is_empty() {
            return Some(0);
        }
        match self.names.get(name).map(|entry| entry.first) {
            Some(first) if first < self.free => Some(first as i32),
            _ => {
                if self.free < self.values.len() {
                    Some(self.free as i32)
                } else {
                    None
                }
            }
        }
    }

    fn advance(&mut self) {
        while self.free < self.values.len()
            && (self.reserved.contains(&self.free) || !self.values[self.free].is_empty())
        {
            self.free += 1;
        }
    }
}

/// Mirror of the `read`/`write` pose of `NativeInputMotion` from donor
/// `src/compat/q2/native-input.ts` (canonical home:
/// `qa_compat::q2::native_input`); unify post-merge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NativeInputPose {
    /// Projected origin.
    pub origin: Vec3,
    /// Projected velocity.
    pub velocity: Vec3,
}

/// Mirror of the `view` result of `NativeInputMotion` from donor
/// `src/compat/q2/native-input.ts` (canonical home:
/// `qa_compat::q2::native_input`); unify post-merge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NativeInputViewState {
    /// Projected view offset.
    pub view_offset: Vec3,
    /// Crouched flag.
    pub crouched: bool,
}

/// Mirror of `NativeInputMotion` from donor
/// `src/compat/q2/native-input.ts` (canonical home:
/// `qa_compat::q2::native_input`); unify post-merge.
pub trait NativeInputMotion {
    /// Read the projected pose.
    fn read(&mut self) -> NativeInputPose;
    /// Whether the projected body is grounded.
    fn grounded(&mut self) -> bool;
    /// Read the projected view.
    fn view(&mut self) -> NativeInputViewState;
    /// Write the projected pose.
    fn write(&mut self, pose: &NativeInputPose);
}

/// Guest print destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuestPrintDestination {
    /// Broadcast print.
    Broadcast,
    /// Debug print.
    Debug,
    /// Client print.
    Client,
    /// Center print.
    Center,
}

/// Resolved model appearance.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelAppearance {
    /// Model path.
    pub path: String,
    /// Skin number.
    pub skin: i32,
    /// Player skin path, if any.
    pub skin_path: Option<String>,
    /// Attached models.
    pub attached_models: Vec<String>,
}

struct ResourceTables {
    model: ResourceNameIndex,
    sound: ResourceNameIndex,
    image: ResourceNameIndex,
}

impl ResourceTables {
    fn classic() -> Self {
        Self {
            model: ResourceNameIndex::new(256, &[]),
            sound: ResourceNameIndex::new(256, &[]),
            image: ResourceNameIndex::new(256, &[]),
        }
    }

    fn get(&self, kind: ResourceKind) -> &ResourceNameIndex {
        match kind {
            ResourceKind::Model => &self.model,
            ResourceKind::Sound => &self.sound,
            ResourceKind::Image => &self.image,
        }
    }

    fn get_mut(&mut self, kind: ResourceKind) -> &mut ResourceNameIndex {
        match kind {
            ResourceKind::Model => &mut self.model,
            ResourceKind::Sound => &mut self.sound,
            ResourceKind::Image => &mut self.image,
        }
    }

    fn clear(&mut self) {
        self.model.clear();
        self.sound.clear();
        self.image.clear();
    }
}

/// API 3 uses the shared actor/collision tables and retains the DLL's
/// public-prefix authority.
pub struct ClassicGuestServices {
    options: ClassicGuestServicesOptions,
    configstrings: HashMap<i32, String>,
    resource_names: ResourceTables,
    messages: Vec<ClassicGuestMessage>,
    buffer: MsgWriter,
    bound: bool,
    combat: Option<ClassicCombatBindings>,
    inventory: Option<ClassicSourceInventory>,
    write_player_velocity: PlayerVelocityWriteFn,
    read_player_velocity: PlayerVelocityReadFn,
    input_motion: HashMap<ActorId, Box<dyn NativeInputMotion>>,
    loading: bool,
}

type PlayerVelocityWriteFn = Option<PlayerVelocityWrite>;
type PlayerVelocityReadFn = Option<PlayerVelocityRead>;

fn msg_write(result: Result<(), MsgError>) -> ClassicResult<()> {
    result.map_err(|error| ClassicQ2Error::invalid(format!("API 3 message write failed: {error}")))
}

fn write_byte_wrapped(writer: &mut MsgWriter, value: f64) -> ClassicResult<()> {
    msg_write(writer.write_byte((float_to_wrapped_i32(value) & 0xff) as u8))
}

/// Donor `stringToBytesNulTerminated`: masked UTF-16 units plus NUL.
fn write_donor_string(writer: &mut MsgWriter, text: &str) -> ClassicResult<()> {
    let mut bytes: Vec<u8> = text.encode_utf16().map(|unit| (unit & 0xff) as u8).collect();
    bytes.push(0);
    msg_write(writer.write_bytes(&bytes))
}

fn classic_number(values: &[GuestCallValue], index: usize) -> ClassicResult<f64> {
    match values.get(index) {
        Some(GuestCallValue::Int32(value)) => Ok(f64::from(*value)),
        Some(GuestCallValue::Uint32(value)) => Ok(f64::from(*value)),
        Some(GuestCallValue::Float32(value)) => Ok(f64::from(*value)),
        Some(GuestCallValue::Float64(value)) => Ok(*value),
        _ => Err(ClassicQ2Error::invalid(format!(
            "API 3 argument {index} must be numeric"
        ))),
    }
}

fn classic_pointer(values: &[GuestCallValue], index: usize) -> ClassicResult<Option<GuestAddress>> {
    match values.get(index) {
        Some(GuestCallValue::Pointer(value)) => Ok(*value),
        _ => Err(ClassicQ2Error::invalid(format!(
            "API 3 argument {index} must be a pointer"
        ))),
    }
}

fn classic_required_pointer(values: &[GuestCallValue], index: usize) -> ClassicResult<GuestAddress> {
    classic_pointer(values, index)?
        .ok_or_else(|| ClassicQ2Error::invalid(format!("API 3 argument {index} cannot be null")))
}

fn mem_i32(memory: &mut SparseGuestMemory, base: GuestAddress, offset: i64) -> ClassicResult<i32> {
    Ok(memory.read_i32(memory.offset(base, offset)?)?)
}

fn mem_u8(memory: &mut SparseGuestMemory, base: GuestAddress, offset: i64) -> ClassicResult<u8> {
    Ok(memory.read_u8(memory.offset(base, offset)?)?)
}

fn mem_i16(memory: &mut SparseGuestMemory, base: GuestAddress, offset: i64) -> ClassicResult<i16> {
    Ok(memory.read_i16(memory.offset(base, offset)?)?)
}

fn mem_u32(memory: &mut SparseGuestMemory, base: GuestAddress, offset: i64) -> ClassicResult<u32> {
    Ok(memory.read_u32(memory.offset(base, offset)?)?)
}

fn mem_f32(memory: &mut SparseGuestMemory, base: GuestAddress, offset: i64) -> ClassicResult<f32> {
    Ok(memory.read_f32(memory.offset(base, offset)?)?)
}

fn mem_shorts3(memory: &mut SparseGuestMemory, base: GuestAddress, offset: i64) -> ClassicResult<[i16; 3]> {
    Ok([
        mem_i16(memory, base, offset)?,
        mem_i16(memory, base, offset + 2)?,
        mem_i16(memory, base, offset + 4)?,
    ])
}

fn mem_vector(memory: &mut SparseGuestMemory, base: GuestAddress, offset: i64) -> ClassicResult<Vec3> {
    Ok(memory.read_f32x3(memory.offset(base, offset)?)?)
}

fn store_vector(memory: &mut SparseGuestMemory, base: GuestAddress, offset: i64, value: Vec3) -> ClassicResult<()> {
    memory.write_f32(memory.offset(base, offset)?, value.x)?;
    memory.write_f32(memory.offset(base, offset + 4)?, value.y)?;
    memory.write_f32(memory.offset(base, offset + 8)?, value.z)?;
    Ok(())
}

fn q2_vec(value: Vec3) -> Q2Vec3 {
    Q2Vec3 {
        x: f64::from(value.x),
        y: f64::from(value.y),
        z: f64::from(value.z),
    }
}

const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };

impl ClassicGuestServices {
    /// Create services over the given options.
    pub fn new(mut options: ClassicGuestServicesOptions) -> ClassicResult<Self> {
        if options.max_clients < 1 || options.max_clients > 256 {
            return Err(ClassicQ2Error::invalid("Invalid API 3 maxclients"));
        }
        if options.scene.model_count() > 255 {
            return Err(ClassicQ2Error::invalid("API 3 map exceeds MAX_MODELS"));
        }
        options
            .cvars
            .set("maxclients", &options.max_clients.to_string(), true)
            .map_err(|error| ClassicQ2Error::invalid(format!("API 3 maxclients cvar failed: {error}")))?;
        let mut services = Self {
            options,
            configstrings: HashMap::new(),
            resource_names: ResourceTables::classic(),
            messages: Vec::new(),
            buffer: MsgWriter::new(1400, false),
            bound: false,
            combat: None,
            inventory: None,
            write_player_velocity: None,
            read_player_velocity: None,
            input_motion: HashMap::new(),
            loading: true,
        };
        let map_path = services.options.map_path.clone();
        services.store_configstring(33, &map_path);
        let models = services.options.scene.model_count();
        for model in 1..models {
            services.store_configstring(33 + model as i32, &format!("*{model}"));
        }
        Ok(services)
    }

    /// Current options.
    #[must_use]
    pub fn options(&self) -> &ClassicGuestServicesOptions {
        &self.options
    }

    /// Run `run` with an input-motion projection installed for `actor`.
    pub fn with_input_movement<T>(
        &mut self,
        actor: &ActorId,
        projection: Box<dyn NativeInputMotion>,
        run: impl FnOnce() -> T,
    ) -> T {
        let previous = self.input_motion.insert(actor.clone(), projection);
        let result = run();
        match previous {
            None => {
                self.input_motion.remove(actor);
            }
            Some(projection) => {
                self.input_motion.insert(actor.clone(), projection);
            }
        }
        result
    }

    /// Install the player velocity writer/reader pair.
    pub fn set_player_velocity_writer(&mut self, write: PlayerVelocityWrite, read: PlayerVelocityRead) {
        self.write_player_velocity = Some(write);
        self.read_player_velocity = Some(read);
    }

    /// Run `run` with equipment movement installed for the host's next pmove.
    ///
    /// The donor consumes the pending movement on the first pmove import of
    /// the scope; the Rust host reads `pmove_options` on every pmove run, so
    /// the movement stays installed for the whole scope. Scopes wrap a single
    /// `ClientThink`, which runs pmove once.
    pub fn with_player_movement<T>(
        &mut self,
        host: &mut ClassicQ2GuestHost,
        movement: Option<EquipmentMovement>,
        run: impl FnOnce(&mut ClassicQ2GuestHost) -> T,
    ) -> T {
        let previous = host.services.pmove_options.equipment;
        host.services.pmove_options.equipment = movement;
        host.services.pmove_options.air_accelerate = f64::from(self.options.cvars.variable_value("sv_airaccelerate"));
        let result = run(host);
        host.services.pmove_options.equipment = previous;
        result
    }

    /// Configured client capacity (donor `services.options.maxClients`).
    #[must_use]
    pub fn max_clients(&self) -> u32 {
        self.options.max_clients
    }

    /// Console variable value (donor `services.options.cvars.variableValue`).
    #[must_use]
    pub fn cvar_value(&self, name: &str) -> f32 {
        self.options.cvars.variable_value(name)
    }

    /// Guest console variables.
    #[must_use]
    pub fn cvars(&self) -> &CvarRegistry {
        &self.options.cvars
    }

    /// Mutable guest console variables.
    pub fn cvars_mut(&mut self) -> &mut CvarRegistry {
        &mut self.options.cvars
    }

    /// Release every engine actor owned by `provider` (map travel cleanup).
    pub fn release_provider_actors(&mut self, provider: &ProviderId) {
        let owned: Vec<ActorId> = self
            .options
            .engine
            .actors()
            .observations()
            .into_iter()
            .filter(|observation| observation.owner == *provider)
            .map(|observation| observation.id)
            .collect();
        for id in owned {
            if let Some(actor) = self.options.engine.actors().resolve_owned(&id) {
                self.options.engine.actors().release(&actor);
            }
        }
    }

    /// Validate a map binding.
    pub fn validate_map(binding: &ClassicGuestMapServices) -> ClassicResult<()> {
        if binding.scene.model_count() > 255 {
            return Err(ClassicQ2Error::invalid("API 3 map exceeds MAX_MODELS"));
        }
        Ok(())
    }

    /// Rebind the services to a new world.
    pub fn rebind_world(&mut self, binding: ClassicGuestMapServices) -> ClassicResult<()> {
        Self::validate_map(&binding)?;
        self.options.engine = binding.engine;
        self.options.scene = binding.scene;
        self.options.map_path = binding.map_path;
        self.options.admit = binding.admit;
        self.options.collision = binding.collision;
        self.options.print = binding.print;
        self.options.command = binding.command;
        self.options.add_command = binding.add_command;
        self.options.debug_graph = binding.debug_graph;
        self.options.damage_provenance = binding.damage_provenance;
        self.options.pickups = binding.pickups;
        self.loading = true;
        self.configstrings.clear();
        self.messages.clear();
        self.buffer.clear();
        self.resource_names.clear();
        let map_path = self.options.map_path.clone();
        self.store_configstring(33, &map_path);
        let models = self.options.scene.model_count();
        for model in 1..models {
            self.store_configstring(33 + model as i32, &format!("*{model}"));
        }
        Ok(())
    }

    /// Bind the services to a guest host.
    ///
    /// The donor also rejects a memory mismatch; the Rust host owns its
    /// memory, so only double binding is rejected. The declared native
    /// pickup profile has no Rust consumer (`bind_pickups` takes the
    /// classic profile), so binding resolves the artifact profile by
    /// digest; see the lane report.
    pub fn bind_host(&mut self, host: &mut ClassicQ2GuestHost, image_base: Option<GuestAddress>) -> ClassicResult<()> {
        if self.bound {
            return Err(ClassicQ2Error::invalid(
                "API 3 services already bound or guest memory mismatch",
            ));
        }
        self.bound = true;
        let primary = self.options.primary_world.clone();
        self.combat = match image_base {
            None => None,
            Some(image) => {
                ClassicCombatBindings::create(host, image, primary.as_ref().map(|profile| profile.combat.clone()))?
            }
        };
        self.inventory = match image_base {
            None => None,
            Some(image) => ClassicSourceInventory::create(host, image, primary)?,
        };
        if let Some(image) = image_base {
            if self.options.pickups.is_some() {
                host.bind_pickups(image, None)?;
            }
        }
        host.services.numeric = self.options.numeric;
        host.services.pmove_options.air_accelerate = f64::from(self.options.cvars.variable_value("sv_airaccelerate"));
        Ok(())
    }

    /// Release combat bindings.
    pub fn dispose(&mut self) {
        if let Some(combat) = self.combat.as_mut() {
            combat.close();
        }
        self.combat = None;
    }

    /// Whether source inventory is available.
    #[must_use]
    pub fn has_source_inventory(&self) -> bool {
        self.inventory.is_some()
    }

    /// Bind the source inventory for a slot.
    pub fn source_inventory(
        &self,
        host: &mut ClassicQ2GuestHost,
        slot: u32,
    ) -> ClassicResult<Option<ClassicInventoryBinding<'_>>> {
        self.inventory
            .as_ref()
            .map(|inventory| inventory.bind(host, slot))
            .transpose()
    }

    /// Read the notarget flag for a slot.
    pub fn notarget(&self, host: &mut ClassicQ2GuestHost, slot: u32) -> ClassicResult<Option<bool>> {
        self.combat
            .as_ref()
            .map(|combat| combat.notarget(host, slot))
            .transpose()
    }

    /// Mark spawning complete.
    pub fn complete_spawn(&mut self) {
        self.loading = false;
    }

    /// Snapshot the configstrings.
    #[must_use]
    pub fn configstrings(&self) -> HashMap<i32, String> {
        self.configstrings.clone()
    }

    /// Drain the queued messages.
    pub fn drain_messages(&mut self) -> Vec<ClassicGuestMessage> {
        std::mem::take(&mut self.messages)
    }

    /// Read a resource configstring.
    #[must_use]
    pub fn resource(&self, kind: ResourceKind, index: i32) -> String {
        self.configstrings
            .get(&(kind.base() + index))
            .cloned()
            .unwrap_or_default()
    }

    /// Resolve a resource index, publishing the configstring on change.
    pub fn resource_index(
        &mut self,
        host: &mut ClassicQ2GuestHost,
        kind: ResourceKind,
        name: &str,
    ) -> ClassicResult<i32> {
        let index = self
            .resource_names
            .get(kind)
            .find(name)
            .ok_or_else(|| ClassicQ2Error::invalid(format!("API 3 {} index overflow", kind.name())))?;
        if index != 0 && self.resource(kind, index) != name {
            self.set_configstring(host, kind.base() + index, name)?;
        }
        Ok(index)
    }

    fn store_configstring(&mut self, index: i32, value: &str) {
        self.configstrings.insert(index, value.to_string());
        for kind in [ResourceKind::Model, ResourceKind::Sound, ResourceKind::Image] {
            let slot = index - kind.base();
            if slot > 0 && slot < 256 {
                self.resource_names.get_mut(kind).set(slot, value);
                break;
            }
        }
    }

    /// Publish a configstring.
    pub fn set_configstring(&mut self, host: &mut ClassicQ2GuestHost, index: i32, value: &str) -> ClassicResult<()> {
        if !(0..2080).contains(&index) {
            return Err(ClassicQ2Error::invalid(
                "API 3 configstring index outside MAX_CONFIGSTRINGS",
            ));
        }
        self.store_configstring(index, value);
        if index > 32 && index < 288 {
            host.set_model_name(index - 32, value)?;
        }
        if (800..1056).contains(&index) {
            self.options.engine.emit(Q2PresentationEvent::LightStyle {
                style: index - 800,
                pattern: value.to_string(),
            });
        }
        if index == 1 {
            self.options.engine.emit(Q2PresentationEvent::Music {
                track: value.to_string(),
            });
        }
        if !self.loading {
            self.buffer.clear();
            msg_write(self.buffer.write_byte(13))?;
            msg_write(self.buffer.write_short(index as i16))?;
            write_donor_string(&mut self.buffer, value)?;
            self.multicast(ZERO, 3)?;
        }
        Ok(())
    }

    fn enqueue(&mut self, audience: ClassicGuestAudience, reliable: bool) {
        self.messages.push(ClassicGuestMessage {
            audience,
            reliable,
            bytes: self.buffer.bytes().to_vec(),
        });
        self.buffer.clear();
    }

    fn multicast(&mut self, origin: Vec3, destination: i32) -> ClassicResult<()> {
        if !(0..=5).contains(&destination) {
            return Err(ClassicQ2Error::invalid("API 3 invalid multicast destination"));
        }
        let scope = match destination % 3 {
            0 => MulticastScope::All,
            1 => MulticastScope::Phs,
            _ => MulticastScope::Pvs,
        };
        self.enqueue(ClassicGuestAudience::Multicast { origin, scope }, destination >= 3);
        Ok(())
    }

    fn unicast(
        &mut self,
        host: &mut ClassicQ2GuestHost,
        entity: Option<GuestAddress>,
        reliable: bool,
    ) -> ClassicResult<()> {
        let Some(entity) = entity else {
            return Ok(());
        };
        let slot = host
            .edicts
            .as_mut()
            .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?
            .record_from_pointer(&mut host.memory, entity)?
            .slot;
        if !self.accepts_client(slot) {
            return Ok(());
        }
        self.enqueue(ClassicGuestAudience::Unicast { slot }, reliable);
        Ok(())
    }

    fn accepts_client(&self, slot: u32) -> bool {
        match &self.options.accepts_client {
            Some(accepts) => accepts(slot),
            None => slot >= 1 && slot <= self.options.max_clients,
        }
    }

    /// Handle one message import.
    pub fn message(
        &mut self,
        host: &mut ClassicQ2GuestHost,
        operation: &str,
        values: &[GuestCallValue],
    ) -> ClassicResult<()> {
        match operation {
            "WriteChar" => {
                let number = classic_number(values, 0)?;
                write_byte_wrapped(&mut self.buffer, number)?;
            }
            "WriteByte" => {
                let number = classic_number(values, 0)?;
                write_byte_wrapped(&mut self.buffer, number)?;
            }
            "WriteShort" => {
                let number = classic_number(values, 0)?;
                msg_write(self.buffer.write_short(float_to_wrapped_i32(number) as i16))?;
            }
            "WriteLong" => {
                let number = classic_number(values, 0)?;
                msg_write(self.buffer.write_long(float_to_wrapped_i32(number)))?;
            }
            "WriteFloat" => {
                let number = classic_number(values, 0)?;
                msg_write(self.buffer.write_float(number as f32))?;
            }
            "WriteString" => {
                let address = classic_pointer(values, 0)?;
                let text = match address {
                    None => String::new(),
                    Some(address) => read_classic_string(&mut host.memory, Some(address), 65536)?,
                };
                write_donor_string(&mut self.buffer, &text)?;
            }
            "WritePosition" => {
                let address = classic_required_pointer(values, 0)?;
                let vector = read_classic_vector(&mut host.memory, address)?;
                msg_write(
                    self.buffer
                        .write_q2_pos([f64::from(vector.x), f64::from(vector.y), f64::from(vector.z)]),
                )?;
            }
            "WriteDir" => {
                let address = classic_pointer(values, 0)?;
                let direction = match address {
                    None => None,
                    Some(address) => {
                        let vector = read_classic_vector(&mut host.memory, address)?;
                        Some([f64::from(vector.x), f64::from(vector.y), f64::from(vector.z)])
                    }
                };
                msg_write(write_dir(&mut self.buffer, direction))?;
            }
            "WriteAngle" => {
                let number = classic_number(values, 0)?;
                msg_write(self.buffer.write_q2_angle(number))?;
            }
            "multicast" => {
                let address = classic_required_pointer(values, 0)?;
                let origin = read_classic_vector(&mut host.memory, address)?;
                let destination = classic_number(values, 1)?;
                if !(0.0..=5.0).contains(&destination) || destination.fract() != 0.0 {
                    return Err(ClassicQ2Error::invalid("API 3 invalid multicast destination"));
                }
                self.multicast(origin, destination as i32)?;
            }
            "unicast" => {
                let entity = classic_pointer(values, 0)?;
                let reliable = classic_number(values, 1)? != 0.0;
                self.unicast(host, entity, reliable)?;
            }
            _ => {
                return Err(ClassicQ2Error::invalid(format!(
                    "Invalid API 3 message operation {operation}"
                )));
            }
        }
        Ok(())
    }

    /// Handle one print import.
    pub fn print(
        &mut self,
        host: &mut ClassicQ2GuestHost,
        destination: GuestPrintDestination,
        entity: Option<GuestAddress>,
        level: i32,
        text: &str,
    ) -> ClassicResult<()> {
        if destination == GuestPrintDestination::Debug
            || destination == GuestPrintDestination::Client && entity.is_none()
        {
            (self.options.print)(text);
            return Ok(());
        }
        if destination != GuestPrintDestination::Broadcast && entity.is_none() {
            return Ok(());
        }
        if destination != GuestPrintDestination::Broadcast {
            let slot = host
                .edicts
                .as_mut()
                .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?
                .record_from_pointer(&mut host.memory, entity.expect("print entity"))?
                .slot;
            if !self.accepts_client(slot) {
                if destination == GuestPrintDestination::Center {
                    return Ok(());
                }
                return Err(ClassicQ2Error::invalid("API 3 cprintf on non-client edict"));
            }
        }
        let mut packet = MsgWriter::new(1400, false);
        msg_write(packet.write_byte(if destination == GuestPrintDestination::Center {
            15
        } else {
            10
        }))?;
        if destination != GuestPrintDestination::Center {
            msg_write(packet.write_byte((level & 0xff) as u8))?;
        }
        write_donor_string(&mut packet, text)?;
        let audience = if destination == GuestPrintDestination::Broadcast {
            ClassicGuestAudience::Multicast {
                origin: ZERO,
                scope: MulticastScope::All,
            }
        } else {
            let slot = host
                .edicts
                .as_mut()
                .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?
                .record_from_pointer(&mut host.memory, entity.expect("print entity"))?
                .slot;
            ClassicGuestAudience::Unicast { slot }
        };
        self.messages.push(ClassicGuestMessage {
            audience,
            reliable: true,
            bytes: packet.bytes().to_vec(),
        });
        if destination == GuestPrintDestination::Broadcast {
            (self.options.print)(text);
        }
        Ok(())
    }

    /// Handle one sound import.
    #[allow(clippy::too_many_arguments)]
    pub fn sound(
        &mut self,
        host: &mut ClassicQ2GuestHost,
        explicit_origin: Option<Vec3>,
        entity: Option<GuestAddress>,
        channel: i32,
        index: i32,
        volume: f64,
        attenuation: f64,
        offset: f64,
    ) -> ClassicResult<()> {
        if !(0.0..=1.0).contains(&volume) || !(0.0..=4.0).contains(&attenuation) || !(0.0..=0.255).contains(&offset) {
            return Err(ClassicQ2Error::invalid("API 3 sound parameters outside source ranges"));
        }
        let Some(entity) = entity else {
            return Err(ClassicQ2Error::invalid("API 3 sound requires an entity"));
        };
        let record = host
            .edicts
            .as_mut()
            .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?
            .record_from_pointer(&mut host.memory, entity)?;
        let body = self.read_body(host, &record)?;
        let solid = mem_i32(&mut host.memory, record.address, 248)?;
        let flags = mem_i32(&mut host.memory, record.address, 184)?;
        let origin = explicit_origin.unwrap_or({
            if solid == 3 {
                Vec3 {
                    x: body.origin.x + (body.bounds.min.x + body.bounds.max.x) * 0.5,
                    y: body.origin.y + (body.bounds.min.y + body.bounds.max.y) * 0.5,
                    z: body.origin.z + (body.bounds.min.z + body.bounds.max.z) * 0.5,
                }
            } else {
                body.origin
            }
        });
        let positioned = explicit_origin.is_some() || flags & 1 != 0 || solid == 3;
        let flags_byte = 8
            | (if volume != 1.0 { 1 } else { 0 })
            | (if attenuation != 1.0 { 2 } else { 0 })
            | (if positioned { 4 } else { 0 })
            | (if offset != 0.0 { 16 } else { 0 });
        msg_write(self.buffer.write_byte(9))?;
        msg_write(self.buffer.write_byte(flags_byte))?;
        msg_write(self.buffer.write_byte(index as u8))?;
        if flags_byte & 1 != 0 {
            write_byte_wrapped(&mut self.buffer, volume * 255.0)?;
        }
        if flags_byte & 2 != 0 {
            write_byte_wrapped(&mut self.buffer, attenuation * 64.0)?;
        }
        if flags_byte & 16 != 0 {
            write_byte_wrapped(&mut self.buffer, offset * 1000.0)?;
        }
        msg_write(
            self.buffer
                .write_short(((record.slot << 3) | (channel as u32 & 7)) as i16),
        )?;
        if positioned {
            msg_write(
                self.buffer
                    .write_q2_pos([f64::from(origin.x), f64::from(origin.y), f64::from(origin.z)]),
            )?;
        }
        self.multicast(
            origin,
            (if channel & 8 != 0 || attenuation == 0.0 { 0 } else { 1 })
                + (if channel & 8 == 0 && channel & 16 != 0 { 3 } else { 0 }),
        )?;
        Ok(())
    }

    fn current_actor(host: &mut ClassicQ2GuestHost, record: &RawEntityView) -> ClassicResult<Option<OwnedActor>> {
        let edicts = host
            .edicts
            .as_mut()
            .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?;
        edicts.current(&mut host.memory, &host.registry, record)
    }

    fn client_prefix(host: &mut ClassicQ2GuestHost, slot: u32) -> ClassicResult<Option<GuestAddress>> {
        let edicts = host
            .edicts
            .as_mut()
            .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?;
        edicts.client_prefix_address(&mut host.memory, slot)
    }

    /// Read the live body projection for a record.
    pub fn read_body(
        &mut self,
        host: &mut ClassicQ2GuestHost,
        record: &RawEntityView,
    ) -> ClassicResult<WorldBodyState> {
        let client = if record.slot >= 1 && record.slot <= self.options.max_clients {
            Self::client_prefix(host, record.slot)?
        } else {
            None
        };
        let actor = Self::current_actor(host, record)?;
        let pending = match (&actor, self.read_player_velocity.as_mut()) {
            (Some(actor), Some(read)) => read(actor.id()),
            _ => None,
        };
        let motion = match actor.as_ref() {
            None => None,
            Some(actor) => self.input_motion.get_mut(actor.id()).map(|motion| motion.read()),
        };
        let velocity = motion
            .map(|pose| pose.velocity)
            .or(pending)
            .or(match client {
                None => None,
                Some(client) => Some(Vec3 {
                    x: mem_i16(&mut host.memory, client, 10)? as f32 * 0.125,
                    y: mem_i16(&mut host.memory, client, 12)? as f32 * 0.125,
                    z: mem_i16(&mut host.memory, client, 14)? as f32 * 0.125,
                }),
            })
            .unwrap_or(ZERO);
        Ok(WorldBodyState {
            origin: match motion {
                Some(pose) => pose.origin,
                None => mem_vector(&mut host.memory, record.address, 4)?,
            },
            angles: mem_vector(&mut host.memory, record.address, 16)?,
            velocity,
            bounds: Bounds {
                min: mem_vector(&mut host.memory, record.address, 188)?,
                max: mem_vector(&mut host.memory, record.address, 200)?,
            },
            ground: None,
        })
    }

    /// Write a body state through to the guest record.
    pub fn write_body(
        &mut self,
        host: &mut ClassicQ2GuestHost,
        record: &RawEntityView,
        actor: &OwnedActor,
        state: &WorldBodyState,
    ) -> ClassicResult<()> {
        let velocity = self.read_body(host, record)?.velocity;
        let actor_id = actor.id().clone();
        if self.input_motion.contains_key(&actor_id) {
            let motion = self.input_motion.get_mut(&actor_id).expect("motion present");
            motion.write(&NativeInputPose {
                origin: state.origin,
                velocity: state.velocity,
            });
        } else {
            if state.velocity != velocity {
                if record.slot < 1 || record.slot > self.options.max_clients || self.write_player_velocity.is_none() {
                    return Err(ClassicQ2Error::invalid(
                        "API3 velocity writes require a source semantic binding",
                    ));
                }
                if let Some(write) = self.write_player_velocity.as_mut() {
                    write(actor, state.velocity);
                }
            }
            store_vector(&mut host.memory, record.address, 4, state.origin)?;
        }
        store_vector(&mut host.memory, record.address, 16, state.angles)?;
        store_vector(&mut host.memory, record.address, 188, state.bounds.min)?;
        store_vector(&mut host.memory, record.address, 200, state.bounds.max)?;
        Ok(())
    }

    /// Bind an engine actor to a guest record.
    ///
    /// The donor installs a live body projection; the Rust body table is
    /// snapshot-based, so the current projection is snapshotted with
    /// `create` and the live read/write path stays available through
    /// [`Self::read_body`]/[`Self::write_body`]. The combat bind has no
    /// Rust equivalent (`ClassicCombatBindings` exposes no per-actor bind);
    /// see the lane report.
    pub fn bind_entity(
        &mut self,
        host: &mut ClassicQ2GuestHost,
        record: &RawEntityView,
        actor: &OwnedActor,
    ) -> ClassicResult<()> {
        let body = self.read_body(host, record)?;
        let snapshot = EngineBodyState {
            origin: body.origin,
            angles: body.angles,
            velocity: body.velocity,
            bounds: body.bounds,
            ground: body.ground,
        };
        self.options.engine.bodies().create(actor, &snapshot);
        (self.options.admit)(record, actor);
        Ok(())
    }

    /// Link a bound body, reporting its collision.
    ///
    /// The link-state restore has no Rust equivalent (`Q2BodyTable`
    /// exposes no `restore_link_state`); see the lane report.
    pub fn link_body(
        &mut self,
        host: &mut ClassicQ2GuestHost,
        record: &RawEntityView,
        actor: &OwnedActor,
    ) -> ClassicResult<()> {
        let solid = mem_i32(&mut host.memory, record.address, 248)?;
        let flags = mem_i32(&mut host.memory, record.address, 184)?;
        let owner_address = host.memory.read_pointer(host.memory.offset(record.address, 256)?)?;
        let owner = match owner_address {
            None => None,
            Some(address) => {
                let edicts = host
                    .edicts
                    .as_mut()
                    .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?;
                edicts
                    .observe(&mut host.memory, &mut host.registry, address)?
                    .map(|observation| observation.actor().id().clone())
            }
        };
        let model_name = self.resource(ResourceKind::Model, mem_i32(&mut host.memory, record.address, 40)?);
        let dead_monster = flags & 2 != 0;
        let monster = flags & 4 != 0;
        let collision = ActorCollision {
            family: CollisionFamily::Q2,
            shape: if solid == 3 {
                CollisionShape::Model(inline_model_number(&model_name))
            } else {
                CollisionShape::Box
            },
            contents: if solid == 3 {
                1
            } else if dead_monster {
                0x4000000
            } else {
                0x2000000
            },
            owner,
            role: if solid == 1 {
                CollisionRole::Trigger
            } else {
                CollisionRole::Solid
            },
            monster,
            dead_monster,
            q1_corpse: false,
            q3_owner: None,
        };
        (self.options.collision)(actor, &collision);
        Ok(())
    }

    /// Compute the world link for bounds.
    pub fn world_link(&self, bounds: &Bounds) -> ClassicWorldLink {
        let query = self.options.scene.box_leaves(bounds, 128);
        let mut clusters: Vec<i32> = Vec::new();
        let (mut first, mut second) = (0, 0);
        for leaf in &query.leaves {
            let area = self.options.scene.leaf_area(*leaf);
            let cluster = self.options.scene.leaf_cluster(*leaf);
            if area != 0 {
                if first != 0 && first != area {
                    second = area;
                } else {
                    first = area;
                }
            }
            if cluster >= 0 && !clusters.contains(&cluster) {
                clusters.push(cluster);
            }
        }
        ClassicWorldLink {
            clusters: if query.overflow || query.leaves.len() >= 128 || clusters.len() > 16 {
                None
            } else {
                Some(clusters)
            },
            headnode: query.topnode.unwrap_or(0),
            areas: (first, second),
        }
    }

    /// Query actors touching bounds.
    pub fn box_edicts(&self, bounds: &Bounds, kind: QueryRole) -> Vec<ActorId> {
        self.options
            .scene
            .query_actors(bounds, kind)
            .iter()
            .map(|item| item.body.actor.clone())
            .collect()
    }

    /// Decode the entity state for a slot.
    pub fn entity_state(host: &mut ClassicQ2GuestHost, slot: u32) -> ClassicResult<Q2EntityState> {
        let edicts = host
            .edicts
            .as_mut()
            .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?;
        let record = edicts.at(&mut host.memory, slot)?;
        let memory = &mut host.memory;
        let address = record.address;
        Ok(Q2EntityState {
            number: mem_i32(memory, address, 0)? as u16,
            origin: q2_vec(mem_vector(memory, address, 4)?),
            angles: q2_vec(mem_vector(memory, address, 16)?),
            old_origin: q2_vec(mem_vector(memory, address, 28)?),
            model_indexes: [
                mem_i32(memory, address, 40)? as u16,
                mem_i32(memory, address, 44)? as u16,
                mem_i32(memory, address, 48)? as u16,
                mem_i32(memory, address, 52)? as u16,
            ],
            frame: mem_i32(memory, address, 56)?,
            skin: mem_i32(memory, address, 60)?,
            effects: mem_u32(memory, address, 64)?,
            render_effects: mem_i32(memory, address, 68)? as u32,
            solid: mem_i32(memory, address, 72)? as u32,
            sound: mem_i32(memory, address, 76)? as u16,
            event: mem_i32(memory, address, 80)? as u8,
        })
    }

    /// Decode the player state for a slot.
    pub fn player_state(host: &mut ClassicQ2GuestHost, slot: u32) -> ClassicResult<Q2PlayerState> {
        let client = Self::client_prefix(host, slot)?
            .ok_or_else(|| ClassicQ2Error::invalid("API 3 client slot has no gclient prefix"))?;
        let memory = &mut host.memory;
        let origin = mem_shorts3(memory, client, 4)?;
        let velocity = mem_shorts3(memory, client, 10)?;
        let delta = mem_shorts3(memory, client, 20)?;
        let mut stats = Vec::with_capacity(32);
        for index in 0..32 {
            stats.push(mem_i16(memory, client, 120 + index * 2)?);
        }
        Ok(Q2PlayerState {
            view: Q2PlayerView {
                view_angles: q2_vec(mem_vector(memory, client, 28)?),
                view_offset: q2_vec(mem_vector(memory, client, 40)?),
                kick_angles: q2_vec(mem_vector(memory, client, 52)?),
                gun_angles: q2_vec(mem_vector(memory, client, 64)?),
                gun_offset: q2_vec(mem_vector(memory, client, 76)?),
                gun_index: mem_i32(memory, client, 88)?,
                gun_frame: mem_i32(memory, client, 92)?,
                fov: mem_f32(memory, client, 112)? as u8,
                render_flags: mem_i32(memory, client, 116)? as u8,
                stats,
            },
            movement: Q2MovementState {
                move_type: mem_i32(memory, client, 0)? as u8,
                origin_eighths: origin.map(i32::from),
                velocity_eighths: velocity.map(i32::from),
                flags: i32::from(mem_u8(memory, client, 16)?),
                time: i32::from(mem_u8(memory, client, 17)?),
                gravity: mem_i16(memory, client, 18)?,
                delta_angle_shorts: delta,
            },
            blend: Q2Vec4 {
                x: f64::from(mem_f32(memory, client, 96)?),
                y: f64::from(mem_f32(memory, client, 100)?),
                z: f64::from(mem_f32(memory, client, 104)?),
                w: f64::from(mem_f32(memory, client, 108)?),
            },
        })
    }

    /// Resolve the model appearance for a slot.
    pub fn model_appearance(&self, host: &mut ClassicQ2GuestHost, slot: u32) -> ClassicResult<ModelAppearance> {
        let state = Self::entity_state(host, slot)?;
        if state.model_indexes.iter().all(|index| *index != 255) {
            return Ok(ModelAppearance {
                path: self.resource(ResourceKind::Model, i32::from(state.model_indexes[0])),
                skin: state.skin,
                skin_path: None,
                attached_models: state.model_indexes[1..]
                    .iter()
                    .map(|index| self.resource(ResourceKind::Model, i32::from(*index)))
                    .collect(),
            });
        }
        let value = self
            .configstrings
            .get(&(1312 + (state.skin & 255)))
            .cloned()
            .unwrap_or_else(|| "player\\male/grunt".to_string());
        let appearance = value.find('\\').map_or(value.as_str(), |index| &value[index + 1..]);
        let (model, skin) = match appearance.find('/') {
            None => ("male".to_string(), "grunt".to_string()),
            Some(slash) => (appearance[..slash].to_string(), appearance[slash + 1..].to_string()),
        };
        let mut weapons = vec!["weapon.md2".to_string()];
        for index in 1..256 {
            let name = self.resource(ResourceKind::Model, index);
            if name.starts_with('#') && weapons.len() < 20 {
                weapons.push(name[1..].to_string());
            }
        }
        let weapon = weapons
            .get(((state.skin as u32 >> 8) & 255) as usize)
            .cloned()
            .unwrap_or_else(|| "weapon.md2".to_string());
        let head = state.model_indexes[0] == 255;
        Ok(ModelAppearance {
            path: if head {
                format!("players/{model}/tris.md2")
            } else {
                self.resource(ResourceKind::Model, i32::from(state.model_indexes[0]))
            },
            skin: if head { 0 } else { state.skin },
            skin_path: if head {
                Some(format!("players/{model}/{skin}.pcx"))
            } else {
                None
            },
            attached_models: state.model_indexes[1..]
                .iter()
                .map(|index| {
                    if *index == 255 {
                        format!("players/{model}/{weapon}")
                    } else {
                        self.resource(ResourceKind::Model, i32::from(*index))
                    }
                })
                .collect(),
        })
    }

    /// Publish entity presentation for every owned actor.
    pub fn publish_entities(&mut self, host: &mut ClassicQ2GuestHost) -> ClassicResult<()> {
        let provider = host.provider.clone();
        let owned: Vec<ActorId> = self
            .options
            .engine
            .actors()
            .observations()
            .into_iter()
            .filter(|observation| observation.owner == provider)
            .map(|observation| observation.id)
            .collect();
        for id in owned {
            let address = {
                let edicts = host
                    .edicts
                    .as_mut()
                    .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?;
                edicts.pointer(&mut host.memory, &host.registry, &id)?
            };
            let record = {
                let edicts = host
                    .edicts
                    .as_mut()
                    .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?;
                edicts.record_from_pointer(&mut host.memory, address)?
            };
            if record.slot == 0 {
                continue;
            }
            let state = Self::entity_state(host, record.slot)?;
            let flags = mem_i32(&mut host.memory, record.address, 184)?;
            self.options.engine.emit(Q2PresentationEvent::Visibility {
                actor: id.clone(),
                visible: flags & 1 == 0,
            });
            let appearance = self.model_appearance(host, record.slot)?;
            self.options.engine.emit(Q2PresentationEvent::Model(Q2ModelEvent {
                actor: id.clone(),
                path: appearance.path,
                attached_models: appearance.attached_models,
                frame: state.frame,
                old_frame: state.frame,
                scale: 1.0,
                alpha: if state.render_effects & 32 != 0 { 0.3 } else { 1.0 },
                skin: appearance.skin,
                effects: i64::from(state.effects),
                render_flags: state.render_effects as i32,
            }));
            if state.event != 0 {
                self.options.engine.emit(Q2PresentationEvent::EntityEvent {
                    actor: id,
                    event: i32::from(state.event),
                });
            }
        }
        Ok(())
    }

    fn require_client_actor(
        &mut self,
        host: &mut ClassicQ2GuestHost,
        slot: u32,
        actor: &ActorId,
        what: &'static str,
    ) -> ClassicResult<RawEntityView> {
        let record = {
            let edicts = host
                .edicts
                .as_mut()
                .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?;
            edicts.at(&mut host.memory, slot)?
        };
        let live = self.options.engine.actors().is_live(actor);
        let current = Self::current_actor(host, &record)?;
        if slot < 1 || slot > self.options.max_clients || !live || current.as_ref().map(OwnedActor::id) != Some(actor) {
            return Err(ClassicQ2Error::invalid(what));
        }
        Ok(record)
    }

    /// Read the grounded flag for a client actor.
    pub fn player_grounded(
        &mut self,
        host: &mut ClassicQ2GuestHost,
        slot: u32,
        actor: &ActorId,
    ) -> ClassicResult<bool> {
        self.require_client_actor(
            host,
            slot,
            actor,
            "API3 movement state requires the current client actor",
        )?;
        if let Some(motion) = self.input_motion.get_mut(actor) {
            return Ok(motion.grounded());
        }
        let client = Self::client_prefix(host, slot)?
            .ok_or_else(|| ClassicQ2Error::invalid("API3 movement state has no client"))?;
        Ok(mem_u8(&mut host.memory, client, 16)? & 4 != 0)
    }

    /// Read the player view for a client actor.
    pub fn player_view(
        &mut self,
        host: &mut ClassicQ2GuestHost,
        slot: u32,
        actor: &ActorId,
    ) -> ClassicResult<NativeInputViewState> {
        self.require_client_actor(host, slot, actor, "API3 player view requires the current client actor")?;
        if let Some(motion) = self.input_motion.get_mut(actor) {
            return Ok(motion.view());
        }
        let client = Self::client_prefix(host, slot)?
            .ok_or_else(|| ClassicQ2Error::invalid("API3 player view has no client"))?;
        Ok(NativeInputViewState {
            view_offset: mem_vector(&mut host.memory, client, 40)?,
            crouched: mem_u8(&mut host.memory, client, 16)? & 1 != 0,
        })
    }

    /// Write the view roll for a client actor.
    pub fn set_player_view_roll(
        &mut self,
        host: &mut ClassicQ2GuestHost,
        slot: u32,
        actor: &ActorId,
        roll: f32,
    ) -> ClassicResult<()> {
        let view_angles = match &self.options.primary_world {
            Some(world) => Some(world.combat.client.view_angles),
            None => {
                let digest = host.memory.module().digest.clone();
                classic_combat_profile(&digest).map(|profile| profile.client.view_angles)
            }
        }
        .ok_or_else(|| ClassicQ2Error::invalid("API3 source view writes require a declared private client layout"))?;
        let record =
            self.require_client_actor(host, slot, actor, "API3 source view requires the current client actor")?;
        let client = host
            .memory
            .read_pointer(host.memory.offset(record.address, 84)?)?
            .ok_or_else(|| ClassicQ2Error::invalid("API3 source view has no client"))?;
        host.memory
            .write_f32(host.memory.offset(client, (view_angles + 8) as i64)?, roll)?;
        Ok(())
    }
}

fn inline_model_number(name: &str) -> u32 {
    name.strip_prefix('*')
        .and_then(|number| number.parse::<u32>().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use qa_bots::scene::{
        LeafQueryResult, PointContentsQuery, PointContentsResult, SceneQueries, TraceDetail, TraceHit as BotsTraceHit,
        TraceQuery, TraceResult as BotsTraceResult, VisibilityKind,
    };
    use qa_compat::q2::classic::layout::classic_q2_exports;
    use qa_content::contract::{InventoryEntry, PoweredProtectionState, RegularArmorState};
    use qa_content::q2::foundation::host::{Q2LandmarkCarry, Q2Motion, Q2PlayerViewState, Q2Solid, Q2TraceRequest};
    use qa_content::q2::support::contracts::{
        ActorObservation, BodyAttachment, CombatState, CombatTraitChanges, DamageOutcome, DamageRequest, LinkedBody,
        PowerArmorCells, Q2BspPlane, Q2TraceFields, TraceContact, TraceFamily, TraceHit, TraceResult, TransitionIntent,
    };
    use qa_content::q2::support::tables::{
        Q2ActorRegistry, Q2BodyTable, Q2CallbackTable, Q2CombatAuthority, Q2InventoryTable,
    };
    use qa_core::cmd::Dialect;
    use qa_core::identity::{IdentityOwner, SavedActorId};
    use qa_core::math::Plane;
    use qa_core::numeric::{NumericOps, Q2_DONOR_PROFILE};
    use qa_guest::core::contracts::{
        ContentDigest, GuestAllocationOptions, GuestCallResult, GuestMapOptions, GuestPermissions, ModuleIdentity,
    };
    use qa_world::spatial::QueryRole as SpatialQueryRole;

    use super::*;
    use crate::bootstrap::simulation::source_hosts::ActorHostScene;

    #[derive(Debug, Clone)]
    struct FakeEngineState {
        actors: HashMap<ActorId, (ProviderId, u32, String)>,
        bodies: HashMap<ActorId, EngineBodyState>,
        emitted: Vec<Q2PresentationEvent>,
        next_slot: u32,
    }

    #[derive(Clone)]
    struct FakeEngine {
        owner: Rc<IdentityOwner>,
        shared: Rc<RefCell<FakeEngineState>>,
    }

    impl FakeEngine {
        fn new() -> Self {
            Self {
                owner: Rc::new(IdentityOwner::create("classic-services-test").expect("owner")),
                shared: Rc::new(RefCell::new(FakeEngineState {
                    actors: HashMap::new(),
                    bodies: HashMap::new(),
                    emitted: Vec::new(),
                    next_slot: 1,
                })),
            }
        }

        fn mint(&self, provider: &ProviderId, slot: u32) -> OwnedActor {
            let id = self.owner.actor(slot, 1);
            self.shared
                .borrow_mut()
                .actors
                .insert(id.clone(), (provider.clone(), slot, "test".to_string()));
            self.owner.owned_actor(&id, provider.clone()).expect("owned")
        }

        fn emitted(&self) -> Vec<Q2PresentationEvent> {
            self.shared.borrow().emitted.clone()
        }
    }

    struct FakeActors {
        engine: FakeEngine,
    }

    impl Q2ActorRegistry for FakeActors {
        fn allocate(&mut self, owner: &ProviderId, definition: &str) -> OwnedActor {
            let mut shared = self.engine.shared.borrow_mut();
            let slot = shared.next_slot;
            shared.next_slot += 1;
            let id = self.engine.owner.actor(slot, 1);
            shared
                .actors
                .insert(id.clone(), (owner.clone(), slot, definition.to_string()));
            drop(shared);
            self.engine.owner.owned_actor(&id, owner.clone()).expect("owned")
        }

        fn allocate_at_source(&mut self, owner: &ProviderId, source_slot: u32, definition: &str) -> OwnedActor {
            let id = self.engine.owner.actor(source_slot, 1);
            self.engine
                .shared
                .borrow_mut()
                .actors
                .insert(id.clone(), (owner.clone(), source_slot, definition.to_string()));
            self.engine.owner.owned_actor(&id, owner.clone()).expect("owned")
        }

        fn source_of(&self, actor: &ActorId) -> Option<(ProviderId, u32)> {
            self.engine
                .shared
                .borrow()
                .actors
                .get(actor)
                .cloned()
                .map(|(owner, slot, _)| (owner, slot))
        }

        fn release(&mut self, actor: &OwnedActor) {
            self.engine.shared.borrow_mut().actors.remove(actor.id());
            self.engine.shared.borrow_mut().bodies.remove(actor.id());
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            self.engine.shared.borrow().actors.contains_key(actor)
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            let shared = self.engine.shared.borrow();
            let (owner, _, _) = shared.actors.get(actor)?;
            self.engine.owner.owned_actor(actor, owner.clone()).ok()
        }

        fn observations(&self) -> Vec<ActorObservation> {
            self.engine
                .shared
                .borrow()
                .actors
                .iter()
                .map(|(id, (owner, _, definition))| ActorObservation {
                    id: id.clone(),
                    owner: owner.clone(),
                    definition: definition.clone(),
                })
                .collect()
        }

        fn resolve_saved(&self, saved: SavedActorId) -> Option<OwnedActor> {
            let id = self.engine.owner.actor(saved.slot, saved.generation);
            self.resolve_owned(&id)
        }

        fn reference_saved(&self, saved: SavedActorId) -> ActorId {
            self.engine.owner.actor(saved.slot, saved.generation)
        }

        fn assert_owned(&self, _actor: &OwnedActor) {}
    }

    struct FakeBodies {
        engine: FakeEngine,
    }

    impl Q2BodyTable for FakeBodies {
        fn create(&mut self, actor: &OwnedActor, initial: &EngineBodyState) {
            self.engine
                .shared
                .borrow_mut()
                .bodies
                .insert(actor.id().clone(), initial.clone());
        }

        fn read(&self, actor: &ActorId) -> Option<EngineBodyState> {
            self.engine.shared.borrow().bodies.get(actor).cloned()
        }

        fn write(&mut self, actor: &OwnedActor, state: &EngineBodyState) {
            self.engine
                .shared
                .borrow_mut()
                .bodies
                .insert(actor.id().clone(), state.clone());
        }

        fn attach(&mut self, _actor: &OwnedActor, _attachment: &BodyAttachment) {}

        fn detach(&mut self, _actor: &OwnedActor) {}

        fn attachment(&self, _actor: &ActorId) -> Option<BodyAttachment> {
            None
        }

        fn linked(&self, _actor: &ActorId) -> Option<LinkedBody> {
            None
        }

        fn link(&mut self, _actor: &OwnedActor, _origin: Option<Vec3>) {}

        fn unlink(&mut self, _actor: &OwnedActor) {}
    }

    struct FakeCallbacks;
    impl Q2CallbackTable for FakeCallbacks {
        fn bind(&mut self, _actor: &OwnedActor) {}
        fn unbind(&mut self, _actor: &ActorId) {}
        fn is_bound(&self, _actor: &ActorId) -> bool {
            false
        }
        fn forward_use(&mut self, _actor: &OwnedActor, _other: Option<&ActorId>, _activator: Option<&ActorId>) {}
    }

    struct FakeCombat;
    impl Q2CombatAuthority for FakeCombat {
        fn create(&mut self, _actor: &OwnedActor, _initial: &CombatState) {}
        fn read(&self, _actor: &ActorId) -> Option<CombatState> {
            None
        }
        fn set_health(&mut self, _actor: &OwnedActor, _health: f64) {}
        fn set_armor(&mut self, _actor: &OwnedActor, _armor: &qa_content::contract::ArmorState) {}
        fn set_regular_points(&mut self, _actor: &OwnedActor, _points: f64, _initial: Option<&RegularArmorState>) {}
        fn set_regular_armor(&mut self, _actor: &OwnedActor, _regular: &RegularArmorState) {}
        fn set_powered_protection(&mut self, _actor: &OwnedActor, _powered: &PoweredProtectionState) {}
        fn set_traits(&mut self, _actor: &OwnedActor, _changes: &CombatTraitChanges) {}
        fn bind_power_armor_cells(&mut self, _actor: &OwnedActor, _cells: Box<dyn PowerArmorCells>) {}
        fn apply(&mut self, input: &DamageRequest) -> DamageOutcome {
            DamageOutcome::StaleTarget { request: input.clone() }
        }
    }

    struct FakeInventory {
        entries: HashMap<ActorId, Vec<InventoryEntry>>,
    }

    impl Q2InventoryTable for FakeInventory {
        fn create(&mut self, actor: &OwnedActor, entries: &[InventoryEntry]) {
            self.entries.insert(actor.id().clone(), entries.to_vec());
        }

        fn entries(&self, actor: &ActorId) -> Vec<InventoryEntry> {
            self.entries.get(actor).cloned().unwrap_or_default()
        }

        fn has(&self, actor: &ActorId) -> bool {
            self.entries.contains_key(actor)
        }

        fn count(&self, actor: &ActorId, item: &ItemId) -> f64 {
            self.entries(actor)
                .iter()
                .filter(|entry| &entry.item == item)
                .map(|entry| entry.count)
                .sum()
        }

        fn consume(&mut self, actor: &OwnedActor, item: &ItemId, count: f64) -> bool {
            let have = self.count(actor.id(), item);
            if have < count {
                return false;
            }
            self.give(actor, item, -count);
            true
        }

        fn give(&mut self, actor: &OwnedActor, item: &ItemId, count: f64) -> f64 {
            let entries = self.entries.entry(actor.id().clone()).or_default();
            match entries.iter_mut().find(|entry| &entry.item == item) {
                Some(entry) => {
                    entry.count += count;
                    entry.count
                }
                None => {
                    entries.push(InventoryEntry {
                        item: item.clone(),
                        count,
                        capacity: f64::INFINITY,
                        count_policy: None,
                    });
                    count
                }
            }
        }

        fn configure(&mut self, actor: &OwnedActor, entry: &InventoryEntry) {
            self.entries.entry(actor.id().clone()).or_default().push(entry.clone());
        }

        fn adjust_source_counter(&mut self, actor: &OwnedActor, item: &ItemId, delta: f64) -> f64 {
            self.give(actor, item, delta)
        }
    }

    // The engine holds its tables behind one shared state; the per-call
    // table objects below are thin views over it.
    struct FakeHost {
        engine: FakeEngine,
        actors: FakeActorsView,
        bodies: FakeBodiesView,
        callbacks: FakeCallbacks,
        combat: FakeCombat,
        inventory: FakeInventory,
    }

    struct FakeActorsView {
        engine: FakeEngine,
    }

    struct FakeBodiesView {
        engine: FakeEngine,
    }

    impl Q2ActorRegistry for FakeActorsView {
        fn allocate(&mut self, owner: &ProviderId, definition: &str) -> OwnedActor {
            FakeActors {
                engine: self.engine.clone(),
            }
            .allocate(owner, definition)
        }

        fn allocate_at_source(&mut self, owner: &ProviderId, source_slot: u32, definition: &str) -> OwnedActor {
            FakeActors {
                engine: self.engine.clone(),
            }
            .allocate_at_source(owner, source_slot, definition)
        }

        fn source_of(&self, actor: &ActorId) -> Option<(ProviderId, u32)> {
            FakeActors {
                engine: self.engine.clone(),
            }
            .source_of(actor)
        }

        fn release(&mut self, actor: &OwnedActor) {
            FakeActors {
                engine: self.engine.clone(),
            }
            .release(actor);
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            FakeActors {
                engine: self.engine.clone(),
            }
            .is_live(actor)
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            FakeActors {
                engine: self.engine.clone(),
            }
            .resolve_owned(actor)
        }

        fn observations(&self) -> Vec<ActorObservation> {
            FakeActors {
                engine: self.engine.clone(),
            }
            .observations()
        }

        fn resolve_saved(&self, saved: SavedActorId) -> Option<OwnedActor> {
            FakeActors {
                engine: self.engine.clone(),
            }
            .resolve_saved(saved)
        }

        fn reference_saved(&self, saved: SavedActorId) -> ActorId {
            FakeActors {
                engine: self.engine.clone(),
            }
            .reference_saved(saved)
        }

        fn assert_owned(&self, actor: &OwnedActor) {
            FakeActors {
                engine: self.engine.clone(),
            }
            .assert_owned(actor);
        }
    }

    impl Q2BodyTable for FakeBodiesView {
        fn create(&mut self, actor: &OwnedActor, initial: &EngineBodyState) {
            FakeBodies {
                engine: self.engine.clone(),
            }
            .create(actor, initial);
        }

        fn read(&self, actor: &ActorId) -> Option<EngineBodyState> {
            FakeBodies {
                engine: self.engine.clone(),
            }
            .read(actor)
        }

        fn write(&mut self, actor: &OwnedActor, state: &EngineBodyState) {
            FakeBodies {
                engine: self.engine.clone(),
            }
            .write(actor, state);
        }

        fn attach(&mut self, actor: &OwnedActor, attachment: &BodyAttachment) {
            FakeBodies {
                engine: self.engine.clone(),
            }
            .attach(actor, attachment);
        }

        fn detach(&mut self, actor: &OwnedActor) {
            FakeBodies {
                engine: self.engine.clone(),
            }
            .detach(actor);
        }

        fn attachment(&self, actor: &ActorId) -> Option<BodyAttachment> {
            FakeBodies {
                engine: self.engine.clone(),
            }
            .attachment(actor)
        }

        fn linked(&self, actor: &ActorId) -> Option<LinkedBody> {
            FakeBodies {
                engine: self.engine.clone(),
            }
            .linked(actor)
        }

        fn link(&mut self, actor: &OwnedActor, origin: Option<Vec3>) {
            FakeBodies {
                engine: self.engine.clone(),
            }
            .link(actor, origin);
        }

        fn unlink(&mut self, actor: &OwnedActor) {
            FakeBodies {
                engine: self.engine.clone(),
            }
            .unlink(actor);
        }
    }

    impl Q2FoundationHost for FakeHost {
        fn actors(&mut self) -> &mut dyn Q2ActorRegistry {
            &mut self.actors
        }

        fn bodies(&mut self) -> &mut dyn Q2BodyTable {
            &mut self.bodies
        }

        fn callbacks(&mut self) -> &mut dyn Q2CallbackTable {
            &mut self.callbacks
        }

        fn combat(&mut self) -> &mut dyn Q2CombatAuthority {
            &mut self.combat
        }

        fn inventory(&mut self) -> &mut dyn Q2InventoryTable {
            &mut self.inventory
        }

        fn now(&self) -> f64 {
            0.0
        }

        fn frame_seconds(&self) -> f64 {
            0.1
        }

        fn gravity(&self) -> f64 {
            800.0
        }

        fn random(&mut self) -> f64 {
            0.5
        }

        fn schedule(&mut self, _actor: &OwnedActor, _due_seconds: Option<f64>) {}

        fn touch_triggers(&mut self, _actor: &OwnedActor) {}

        fn trace(&mut self, request: &Q2TraceRequest) -> TraceResult {
            TraceResult {
                fraction: 1.0,
                end: request.end,
                start_solid: false,
                all_solid: false,
                contact: TraceContact::None,
                hit: TraceHit::None,
                family: TraceFamily::Q2(Q2TraceFields {
                    contents: 0,
                    surface: None,
                    source_plane: Q2BspPlane {
                        normal: ZERO,
                        distance: 0.0,
                        plane_type: 0,
                        signbits: 0,
                    },
                    secondary: None,
                }),
            }
        }

        fn point_contents(&mut self, _point: Vec3) -> i32 {
            0
        }

        fn in_pvs(&mut self, _first: Vec3, _second: Vec3) -> bool {
            true
        }

        fn in_phs(&mut self, _first: Vec3, _second: Vec3) -> bool {
            true
        }

        fn areas_connected(&mut self, _first: Vec3, _second: Vec3) -> bool {
            true
        }

        fn nearby(&mut self, _origin: Vec3, _radius: f64) -> Vec<ActorId> {
            Vec::new()
        }

        fn players(&mut self) -> Vec<ActorId> {
            Vec::new()
        }

        fn world_actor(&mut self) -> ActorId {
            self.engine.owner.actor(0, 0)
        }

        fn is_player(&mut self, _actor: &ActorId) -> bool {
            false
        }

        fn is_monster(&mut self, _actor: &ActorId) -> bool {
            false
        }

        fn inline_model_bounds(&mut self, _model: i32) -> Bounds {
            Bounds { min: ZERO, max: ZERO }
        }

        fn set_solid(&mut self, _actor: &OwnedActor, _solid: Q2Solid, _model: Option<i32>) {}

        fn set_motion(&mut self, _motion: &Q2Motion) {}

        fn set_area_portal(&mut self, _portal: i32, _open: bool) {}

        fn emit(&mut self, event: Q2PresentationEvent) {
            self.engine.shared.borrow_mut().emitted.push(event);
        }

        fn player_view_state(&mut self, _player: &ActorId) -> Option<Q2PlayerViewState> {
            None
        }

        fn key_consumed(&mut self, _player: &ActorId) {}

        fn prepare_level_change(&mut self, _map: &str, _landmark: Option<&Q2LandmarkCarry>, _server_flags: i32) {}

        fn transition(&mut self, _intent: TransitionIntent) {}

        fn diagnostic(&mut self, _message: &str) {}
    }

    struct FakeScene {
        models: usize,
    }

    impl SceneQueries for FakeScene {
        fn trace(&self, query: &TraceQuery) -> BotsTraceResult {
            BotsTraceResult {
                fraction: 1.0,
                end: query.end,
                start_solid: false,
                all_solid: false,
                contact: qa_bots::scene::TraceContact::None,
                hit: BotsTraceHit::None,
                detail: TraceDetail::Q1 {
                    in_open: true,
                    in_water: false,
                    source_plane: Plane {
                        normal: ZERO,
                        distance: 0.0,
                    },
                    surface_flags: None,
                    contents: None,
                },
            }
        }

        fn point_contents(&self, _query: &PointContentsQuery) -> PointContentsResult {
            PointContentsResult::Q1 { contents: 0 }
        }

        fn box_leaves(&self, _bounds: &Bounds, _limit: usize) -> LeafQueryResult {
            LeafQueryResult {
                leaves: vec![1],
                topnode: Some(2),
                overflow: false,
            }
        }

        fn areas_connected(&self, _first: i32, _second: i32) -> bool {
            true
        }

        fn cluster_visible(&self, _from: i32, _to: i32, _kind: VisibilityKind) -> bool {
            true
        }
    }

    impl ActorHostScene for FakeScene {
        fn trace_excluding(&self, query: &TraceQuery, _excluded: &[ActorId]) -> BotsTraceResult {
            self.trace(query)
        }

        fn geometry_trace(&self, query: &TraceQuery) -> BotsTraceResult {
            self.trace(query)
        }

        fn point_leaf(&self, _point: Vec3) -> i32 {
            1
        }

        fn leaf_cluster(&self, _leaf: i32) -> i32 {
            2
        }

        fn leaf_area(&self, _leaf: i32) -> i32 {
            3
        }

        fn model_bounds(&self, _model: i32) -> Bounds {
            Bounds { min: ZERO, max: ZERO }
        }

        fn query_actors(&self, _bounds: &Bounds, _role: SpatialQueryRole) -> Vec<qa_world::spatial::SpatialActor> {
            Vec::new()
        }

        fn model_count(&self) -> usize {
            self.models
        }

        fn q2_texture_info(&self) -> Option<Vec<crate::bootstrap::simulation::source_hosts::SceneTextureInfo>> {
            None
        }
    }

    struct Fixture {
        engine: FakeEngine,
        prints: Rc<RefCell<Vec<String>>>,
        admits: Rc<RefCell<Vec<u32>>>,
        collisions: Rc<RefCell<Vec<(u32, i32)>>>,
    }

    fn options(fixture: &Fixture) -> ClassicGuestServicesOptions {
        let prints = fixture.prints.clone();
        let admits = fixture.admits.clone();
        let collisions = fixture.collisions.clone();
        let engine = fixture.engine.clone();
        ClassicGuestServicesOptions {
            primary_world: None,
            pickup_profile: None,
            pickups: None,
            damage_provenance: None,
            engine: Box::new(FakeHost {
                engine: engine.clone(),
                actors: FakeActorsView { engine: engine.clone() },
                bodies: FakeBodiesView { engine },
                callbacks: FakeCallbacks,
                combat: FakeCombat,
                inventory: FakeInventory {
                    entries: HashMap::new(),
                },
            }),
            scene: Rc::new(FakeScene { models: 3 }),
            cvars: CvarRegistry::new(Dialect::Q2Classic),
            numeric: NumericOps::select(Q2_DONOR_PROFILE).expect("numeric"),
            map_path: "maps/test.bsp".to_string(),
            max_clients: 8,
            accepts_client: None,
            admit: Box::new(move |record, _| {
                admits.borrow_mut().push(record.slot);
            }),
            collision: Box::new(move |actor, collision| {
                collisions.borrow_mut().push((actor.id().slot(), collision.contents));
            }),
            print: Box::new(move |text| {
                prints.borrow_mut().push(text.to_string());
            }),
            command: Rc::new(|| GuestCommandLine {
                arguments: Vec::new(),
                args: String::new(),
            }),
            add_command: Box::new(|_| {}),
            debug_graph: Box::new(|_, _| {}),
        }
    }

    fn fixture() -> (Fixture, ClassicGuestServices) {
        let fixture = Fixture {
            engine: FakeEngine::new(),
            prints: Rc::new(RefCell::new(Vec::new())),
            admits: Rc::new(RefCell::new(Vec::new())),
            collisions: Rc::new(RefCell::new(Vec::new())),
        };
        let services = ClassicGuestServices::new(options(&fixture)).expect("services");
        (fixture, services)
    }

    fn test_host() -> ClassicQ2GuestHost {
        ClassicQ2GuestHost::new(
            ProviderId::new("q2", "classic"),
            ModuleIdentity::new(
                ProviderId::new("q2", "classic"),
                "gamex86.dll",
                ContentDigest::new("sha256", "test"),
                "test",
            ),
            100000,
        )
        .expect("host")
    }

    const CODE_BASE: u64 = 0x10000000;

    fn bind_game(host: &mut ClassicQ2GuestHost) {
        host.memory
            .map(&GuestMapOptions {
                base: CODE_BASE,
                byte_length: 0x10000,
                permissions: GuestPermissions::ReadWriteExecute,
                label: "test code".to_string(),
                bytes: None,
            })
            .expect("map");
        let space = host.memory.address_space();
        let game_api = GuestAddress::new(space, CODE_BASE);
        host.register_guest_handler(
            game_api.offset,
            Box::new(move |memory, _| {
                let exports = memory.allocate(&GuestAllocationOptions::bytes(80))?;
                memory.write_i32(exports, 3)?;
                for (index, entry) in classic_q2_exports().iter().enumerate() {
                    let target = GuestAddress::new(space, CODE_BASE + (1 + index as u64) * 0x100);
                    memory.write_pointer(memory.offset(exports, i64::from(entry.offset))?, Some(target))?;
                }
                let edicts = memory.allocate(&GuestAllocationOptions::bytes(896 * 8))?;
                memory.write_pointer(memory.offset(exports, 64)?, Some(edicts))?;
                memory.write_i32(memory.offset(exports, 68)?, 896)?;
                memory.write_i32(memory.offset(exports, 72)?, 8)?;
                memory.write_i32(memory.offset(exports, 76)?, 8)?;
                memory.write_i32(memory.offset(edicts, 88)?, 1)?;
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(exports))))
            }),
        );
        for (index, entry) in classic_q2_exports().iter().enumerate() {
            let address = CODE_BASE + (1 + index as u64) * 0x100;
            if entry.name == "ClientConnect" {
                host.register_guest_handler(
                    address,
                    Box::new(|_, _| Ok(GuestCallResult::Value(GuestCallValue::Int32(1)))),
                );
            } else {
                host.register_guest_handler(address, Box::new(|_, _| Ok(GuestCallResult::Void)));
            }
        }
        host.get_game_api(game_api).expect("game api");
    }

    fn edict_address(host: &mut ClassicQ2GuestHost, slot: u32) -> GuestAddress {
        let edicts = host.edicts.as_mut().expect("edicts");
        edicts.at(&mut host.memory, slot).expect("record").address
    }

    fn write_client(host: &mut ClassicQ2GuestHost, slot: u32) -> GuestAddress {
        let client = host
            .memory
            .allocate(&GuestAllocationOptions::bytes(512))
            .expect("client");
        let edict = edict_address(host, slot);
        host.memory
            .write_pointer(host.memory.offset(edict, 84).expect("off"), Some(client))
            .expect("link");
        host.memory
            .write_i32(host.memory.offset(edict, 88).expect("off"), 1)
            .expect("inuse");
        client
    }

    #[test]
    fn seeds_map_and_model_configstrings() {
        let (_fixture, services) = fixture();
        let strings = services.configstrings();
        assert_eq!(strings.get(&33).cloned().unwrap_or_default(), "maps/test.bsp");
        assert_eq!(strings.get(&34).cloned().unwrap_or_default(), "*1");
        assert_eq!(strings.get(&35).cloned().unwrap_or_default(), "*2");
        assert!(!strings.contains_key(&36));
    }

    #[test]
    fn rejects_invalid_max_clients() {
        let fixture = Fixture {
            engine: FakeEngine::new(),
            prints: Rc::new(RefCell::new(Vec::new())),
            admits: Rc::new(RefCell::new(Vec::new())),
            collisions: Rc::new(RefCell::new(Vec::new())),
        };
        let mut bad = options(&fixture);
        bad.max_clients = 0;
        assert!(ClassicGuestServices::new(bad).is_err());
    }

    #[test]
    fn set_configstring_broadcasts_after_spawn() {
        let (_fixture, mut services) = fixture();
        let mut host = test_host();
        services.set_configstring(&mut host, 10, "hello").expect("set");
        assert!(services.drain_messages().is_empty());
        services.complete_spawn();
        services.set_configstring(&mut host, 10, "world").expect("set");
        let messages = services.drain_messages();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].reliable);
        assert_eq!(
            messages[0].audience,
            ClassicGuestAudience::Multicast {
                origin: ZERO,
                scope: MulticastScope::All
            }
        );
        assert_eq!(&messages[0].bytes[0..3], &[13, 10, 0]);
        assert!(messages[0].bytes.ends_with(b"world\0"));
    }

    #[test]
    fn set_configstring_emits_lightstyle_and_music() {
        let (fixture, mut services) = fixture();
        let mut host = test_host();
        services.complete_spawn();
        services.set_configstring(&mut host, 801, "mm").expect("light");
        services.set_configstring(&mut host, 1, "track2").expect("music");
        let emitted = fixture.engine.emitted();
        assert!(emitted.iter().any(|event| matches!(
            event,
            Q2PresentationEvent::LightStyle { style: 1, pattern } if pattern == "mm"
        )));
        assert!(emitted.iter().any(|event| matches!(
            event,
            Q2PresentationEvent::Music { track } if track == "track2"
        )));
    }

    #[test]
    fn resource_index_allocates_and_republishes() {
        let (_fixture, mut services) = fixture();
        let mut host = test_host();
        // Seeded map + inline models occupy slots 1..=3.
        let first = services
            .resource_index(&mut host, ResourceKind::Model, "models/a.md2")
            .expect("index");
        assert_eq!(first, 4);
        assert_eq!(services.resource(ResourceKind::Model, 4), "models/a.md2");
        let again = services
            .resource_index(&mut host, ResourceKind::Model, "models/a.md2")
            .expect("index");
        assert_eq!(again, 4);
        let second = services
            .resource_index(&mut host, ResourceKind::Model, "models/b.md2")
            .expect("index");
        assert_eq!(second, 5);
    }

    #[test]
    fn message_ops_pack_bytes() {
        let (_fixture, mut services) = fixture();
        let mut host = test_host();
        services
            .message(&mut host, "WriteByte", &[GuestCallValue::Int32(9)])
            .expect("byte");
        services
            .message(&mut host, "WriteShort", &[GuestCallValue::Int32(0x1234)])
            .expect("short");
        services
            .message(&mut host, "WriteString", &[GuestCallValue::Pointer(None)])
            .expect("string");
        services
            .message(
                &mut host,
                "unicast",
                &[GuestCallValue::Pointer(None), GuestCallValue::Int32(1)],
            )
            .expect("unicast-null");
        assert!(services.drain_messages().is_empty());
        services
            .message(&mut host, "WriteByte", &[GuestCallValue::Int32(7)])
            .expect("byte");
        services
            .message(
                &mut host,
                "multicast",
                &[GuestCallValue::Pointer(None), GuestCallValue::Int32(0)],
            )
            .expect_err("null multicast origin");
    }

    #[test]
    fn multicast_scope_mapping_matches_source() {
        let (_fixture, mut services) = fixture();
        for (destination, scope, reliable) in [
            (0, MulticastScope::All, false),
            (1, MulticastScope::Phs, false),
            (2, MulticastScope::Pvs, false),
            (3, MulticastScope::All, true),
            (4, MulticastScope::Phs, true),
            (5, MulticastScope::Pvs, true),
        ] {
            services.multicast(ZERO, destination).expect("multicast");
            let messages = services.drain_messages();
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].reliable, reliable);
            assert_eq!(
                messages[0].audience,
                ClassicGuestAudience::Multicast { origin: ZERO, scope }
            );
        }
        assert!(services.multicast(ZERO, 6).is_err());
    }

    #[test]
    fn print_routes_broadcast_debug_and_drops() {
        let (fixture, mut services) = fixture();
        let mut host = test_host();
        services
            .print(&mut host, GuestPrintDestination::Debug, None, 0, "dbg")
            .expect("debug");
        services
            .print(&mut host, GuestPrintDestination::Broadcast, None, 2, "all")
            .expect("broadcast");
        services
            .print(&mut host, GuestPrintDestination::Client, None, 0, "cli")
            .expect("client-null");
        services
            .print(&mut host, GuestPrintDestination::Center, None, 0, "drop")
            .expect("center-null");
        assert_eq!(*fixture.prints.borrow(), vec!["dbg", "all", "cli"]);
        let messages = services.drain_messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].bytes[0], 10);
        assert_eq!(messages[0].bytes[1], 2);
    }

    #[test]
    fn entity_state_decodes_edict_bytes() {
        let (_fixture, _services) = fixture();
        let mut host = test_host();
        bind_game(&mut host);
        let address = edict_address(&mut host, 1);
        host.memory
            .write_i32(host.memory.offset(address, 0).expect("o"), 7)
            .expect("w");
        host.memory
            .write_f32(host.memory.offset(address, 4).expect("o"), 1.0)
            .expect("w");
        host.memory
            .write_i32(host.memory.offset(address, 56).expect("o"), 9)
            .expect("w");
        host.memory
            .write_i32(host.memory.offset(address, 60).expect("o"), 3)
            .expect("w");
        host.memory
            .write_u32(host.memory.offset(address, 64).expect("o"), 33)
            .expect("w");
        let state = ClassicGuestServices::entity_state(&mut host, 1).expect("state");
        assert_eq!(state.number, 7);
        assert_eq!(state.origin.x, 1.0);
        assert_eq!(state.frame, 9);
        assert_eq!(state.skin, 3);
        assert_eq!(state.effects, 33);
    }

    #[test]
    fn player_state_decodes_client_prefix() {
        let (_fixture, _services) = fixture();
        let mut host = test_host();
        bind_game(&mut host);
        let client = write_client(&mut host, 1);
        host.memory
            .write_i32(host.memory.offset(client, 0).expect("o"), 2)
            .expect("w");
        host.memory
            .write_i16(host.memory.offset(client, 4).expect("o"), 16)
            .expect("w");
        host.memory
            .write_u8(host.memory.offset(client, 16).expect("o"), 4)
            .expect("w");
        host.memory
            .write_i32(host.memory.offset(client, 88).expect("o"), 5)
            .expect("w");
        host.memory
            .write_f32(host.memory.offset(client, 112).expect("o"), 90.0)
            .expect("w");
        host.memory
            .write_i16(host.memory.offset(client, 120).expect("o"), 11)
            .expect("w");
        host.memory
            .write_i16(host.memory.offset(client, 122).expect("o"), 100)
            .expect("w");
        let state = ClassicGuestServices::player_state(&mut host, 1).expect("state");
        assert_eq!(state.movement.move_type, 2);
        assert_eq!(state.movement.origin_eighths[0], 16);
        assert_eq!(state.movement.flags, 4);
        assert_eq!(state.view.gun_index, 5);
        assert_eq!(state.view.fov, 90);
        assert_eq!(state.view.stats[0], 11);
        assert_eq!(state.view.stats[1], 100);
    }

    #[test]
    fn model_appearance_resolves_player_skin() {
        let (_fixture, mut services) = fixture();
        let mut host = test_host();
        bind_game(&mut host);
        let address = edict_address(&mut host, 1);
        host.memory
            .write_i32(host.memory.offset(address, 40).expect("o"), 255)
            .expect("w");
        host.memory
            .write_i32(host.memory.offset(address, 44).expect("o"), 255)
            .expect("w");
        host.memory
            .write_i32(host.memory.offset(address, 60).expect("o"), 0x100)
            .expect("w");
        services
            .set_configstring(&mut host, 1312, "player\\female/athena")
            .expect("skin");
        services.set_configstring(&mut host, 33, "#w_rail.md2").expect("weapon");
        let appearance = services.model_appearance(&mut host, 1).expect("appearance");
        assert_eq!(appearance.path, "players/female/tris.md2");
        assert_eq!(appearance.skin, 0);
        assert_eq!(appearance.skin_path.as_deref(), Some("players/female/athena.pcx"));
        assert_eq!(appearance.attached_models[0], "players/female/w_rail.md2");
    }

    #[test]
    fn world_link_folds_leaves() {
        let (_fixture, services) = fixture();
        let link = services.world_link(&Bounds { min: ZERO, max: ZERO });
        assert_eq!(link.areas, (3, 0));
        assert_eq!(link.headnode, 2);
        assert_eq!(link.clusters, Some(vec![2]));
    }

    #[test]
    fn name_index_matches_source_lookup_order() {
        let mut index = ResourceNameIndex::new(4, &[]);
        assert_eq!(index.find(""), Some(0));
        assert_eq!(index.find("a"), Some(1));
        index.set(1, "a");
        assert_eq!(index.find("a"), Some(1));
        assert_eq!(index.find("b"), Some(2));
        index.set(3, "b");
        assert_eq!(index.find("b"), Some(2));
        index.set(2, "b");
        assert_eq!(index.find("b"), Some(2));
        index.set(1, "");
        assert_eq!(index.find("c"), Some(1));
        let mut reserved = ResourceNameIndex::new(4, &[1]);
        assert_eq!(reserved.find("x"), Some(2));
        reserved.set(1, "x");
        assert_eq!(reserved.find("x"), Some(2));
    }

    #[test]
    fn bind_and_link_report_admission() {
        let (fixture, mut services) = fixture();
        let mut host = test_host();
        bind_game(&mut host);
        let actor = fixture.engine.mint(&ProviderId::new("q2", "classic"), 9);
        let record = {
            let edicts = host.edicts.as_mut().expect("edicts");
            edicts.at(&mut host.memory, 1).expect("record")
        };
        services.bind_entity(&mut host, &record, &actor).expect("bind");
        services.link_body(&mut host, &record, &actor).expect("link");
        assert_eq!(*fixture.admits.borrow(), vec![1]);
        assert_eq!(fixture.collisions.borrow().len(), 1);
    }
}
