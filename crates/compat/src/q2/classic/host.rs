//! Donor: `src/compat/q2/classic/host.ts` — the API 3 guest host.
//!
//! Bridges the game module to engine services: the 44-entry import table,
//! the export-table lifecycle (`GetGameAPI`, `Init`, frame, clients, save),
//! all engine import calls, trace encoding, and entity linking run against
//! synthetic guest handlers and scripted engine answers instead of a real
//! CPU. Async loading twins from the donor collapse into the synchronous
//! calls below; yielding has no meaning without real guest execution.

use std::collections::hash_map::Entry;
use std::collections::HashMap;

use qa_core::identity::{OwnedActor, ProviderId};
use qa_core::math::{Bounds, Vec3};
use qa_core::numeric::{float_to_wrapped_i32, NumericOps, Q2_DONOR_PROFILE};
use qa_guest::abi::values::validate_value_layout;
use qa_guest::core::contracts::{
    GuestAccess, GuestAddress, GuestAllocationOptions, GuestCallResult, GuestCallSignature, GuestCallValue,
    GuestPermissions, GuestValueLayout, ModuleIdentity,
};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::runtime::common::memory::{allocate_native_memory, native_allocation_bytes};
use qa_world::inventory::InventoryEntry;
use qa_world::pickups::{preview_pickup_grants, PickupGrantPlan, PickupSupplyPreview};
use qa_world::registry::ActorRegistry;

use super::cvars::{ClassicCvarRegistry, ClassicQ2Cvars};
use super::layout::{
    classic_q2_export, classic_q2_imports, classic_signature, q2_pointer, q2_trace, ClassicQ2Error, ClassicResult,
    CLASSIC_Q2_IMPORT_BYTES,
};
use super::pickup_profile::{classic_pickup_profile, ClassicPickupProfile};
use super::pmove::{run_classic_guest_pmove, ClassicGuestPmoveOptions, ClassicMovementBody, PmoveEntities, PmoveTrace};
use qa_world::movement::q2::types::{MovementEntity, SrcVec3};
use super::printf::{classic_printf, classic_printf_layouts};
use super::records::{
    allocate_classic_string, classic_string_allocation_bytes, read_classic_string, read_classic_vector,
    write_classic_string, write_classic_vector, ClassicQ2Edicts,
};

/// Synthetic guest function answering one invocation.
pub type GuestHandler = Box<dyn FnMut(&mut SparseGuestMemory, &[GuestCallValue]) -> ClassicResult<GuestCallResult>>;

/// Input-movement boundary wrapping one movement run.
pub type InputMovementBoundary = Box<
    dyn FnMut(GuestAddress, &mut dyn FnMut(Option<&ClassicMovementBody>) -> ClassicResult<()>) -> ClassicResult<()>,
>;

/// Print destination for formatted game text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrintDestination {
    /// Broadcast print.
    Broadcast,
    /// Debug print.
    Debug,
    /// Client print.
    Client,
    /// Center print.
    Center,
}

/// One formatted print record.
#[derive(Debug, Clone, PartialEq)]
pub struct PrintRecord {
    /// Destination.
    pub destination: PrintDestination,
    /// Target entity, if any.
    pub entity: Option<GuestAddress>,
    /// Print level.
    pub level: i32,
    /// Formatted text.
    pub text: String,
}

/// One positional sound record.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundRecord {
    /// Sound origin, if positioned.
    pub origin: Option<Vec3>,
    /// Sound entity, if any.
    pub entity: Option<GuestAddress>,
    /// Channel.
    pub channel: i32,
    /// Sound index.
    pub index: i32,
    /// Volume.
    pub volume: f32,
    /// Attenuation.
    pub attenuation: f32,
    /// Time offset.
    pub time_offset: f32,
}

/// One network message record.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageRecord {
    /// Message operation.
    pub operation: String,
    /// Message values.
    pub values: Vec<GuestCallValue>,
}

/// Scripted world link answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicWorldLink {
    /// Leaf clusters, if known.
    pub clusters: Option<Vec<i32>>,
    /// Head node.
    pub headnode: i32,
    /// Area numbers.
    pub areas: (i32, i32),
}

/// Body link state for one slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyLinkRecord {
    /// Solid kind word.
    pub solid: i32,
    /// Brush model number for brush solids.
    pub brush_model: Option<i32>,
    /// Whether the body is linked.
    pub linked: bool,
}

/// Trace hit reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceHitSlot {
    /// No hit.
    None,
    /// World hit.
    World,
    /// Edict slot hit.
    Slot(u32),
}

/// Trace surface reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicSurface {
    /// Surface name.
    pub name: String,
    /// Surface flags.
    pub flags: i32,
    /// Surface value.
    pub value: i32,
}

/// Scripted trace answer.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassicTraceResult {
    /// Entire sweep inside solid.
    pub all_solid: bool,
    /// Sweep start inside solid.
    pub start_solid: bool,
    /// Travel fraction.
    pub fraction: f32,
    /// Trace end.
    pub end: Vec3,
    /// Impact plane normal.
    pub plane_normal: Vec3,
    /// Impact plane distance.
    pub plane_dist: f32,
    /// Plane type.
    pub plane_type: u8,
    /// Plane sign bits.
    pub plane_signbits: u8,
    /// Contents at the impact.
    pub contents: i32,
    /// Impact surface.
    pub surface: Option<ClassicSurface>,
    /// Hit reference.
    pub hit: TraceHitSlot,
}

/// One trace query log entry.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceQuery {
    /// Sweep start.
    pub start: Vec3,
    /// Sweep end.
    pub end: Vec3,
    /// Hull minimums, if any.
    pub mins: Option<Vec3>,
    /// Hull maximums, if any.
    pub maxs: Option<Vec3>,
    /// Ignored slot, if any.
    pub ignore: Option<u32>,
    /// Contents mask.
    pub mask: i32,
}

/// One box-edicts query log entry.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxEdictsQuery {
    /// Search bounds.
    pub bounds: Bounds,
    /// Search kind.
    pub kind: String,
    /// Maximum count.
    pub maximum: i32,
}

/// Synthetic engine services answering the game imports.
#[derive(Debug, Clone)]
pub struct ClassicEngineServices {
    /// Formatted prints.
    pub prints: Vec<PrintRecord>,
    /// Configstrings by index.
    pub configstrings: HashMap<i32, String>,
    /// Resource indexes by (kind, name).
    pub resources: HashMap<(String, String), i32>,
    /// Next resource index.
    pub next_resource: i32,
    /// Positional sounds.
    pub sounds: Vec<SoundRecord>,
    /// Area portal states.
    pub area_portals: HashMap<i32, bool>,
    /// Scripted area connectivity answers.
    pub areas_connected: HashMap<(i32, i32), bool>,
    /// Scripted world link.
    pub world_link: ClassicWorldLink,
    /// Scripted box-edicts slots.
    pub box_edict_slots: Vec<u32>,
    /// Box-edicts queries.
    pub box_edict_queries: Vec<BoxEdictsQuery>,
    /// Network messages.
    pub messages: Vec<MessageRecord>,
    /// Current command arguments.
    pub command_args: Vec<String>,
    /// Current raw command string.
    pub command_string: String,
    /// Added command strings.
    pub added_commands: Vec<String>,
    /// Debug graph samples.
    pub debug_graphs: Vec<(f32, i32)>,
    /// Scripted trace answer.
    pub trace_result: ClassicTraceResult,
    /// Trace queries.
    pub trace_queries: Vec<TraceQuery>,
    /// Scripted point contents.
    pub point_contents_value: i32,
    /// Scripted PVS answer.
    pub in_pvs_result: bool,
    /// Scripted PHS answer.
    pub in_phs_result: bool,
    /// Scripted inline model bounds.
    pub inline_bounds: Bounds,
    /// Body link states by slot.
    pub body_links: HashMap<u32, BodyLinkRecord>,
    /// Slots passed to the bind-entity service.
    pub bound_entities: Vec<u32>,
    /// Slots linked by the link-entity service.
    pub linked_entities: Vec<u32>,
    /// Slots unlinked by the unlink-entity service.
    pub unlinked_entities: Vec<u32>,
    /// Numeric profile for movement.
    pub numeric: NumericOps,
    /// Movement options for the `Pmove` import.
    pub pmove_options: ClassicGuestPmoveOptions,
}

impl ClassicEngineServices {
    /// Create services with default scripted answers.
    #[must_use]
    pub fn new(numeric: NumericOps) -> Self {
        let zero = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
        Self {
            prints: Vec::new(),
            configstrings: HashMap::new(),
            resources: HashMap::new(),
            next_resource: 1,
            sounds: Vec::new(),
            area_portals: HashMap::new(),
            areas_connected: HashMap::new(),
            world_link: ClassicWorldLink {
                clusters: None,
                headnode: 0,
                areas: (0, 0),
            },
            box_edict_slots: Vec::new(),
            box_edict_queries: Vec::new(),
            messages: Vec::new(),
            command_args: Vec::new(),
            command_string: String::new(),
            added_commands: Vec::new(),
            debug_graphs: Vec::new(),
            trace_result: ClassicTraceResult {
                all_solid: false,
                start_solid: false,
                fraction: 1.0,
                end: zero,
                plane_normal: zero,
                plane_dist: 0.0,
                plane_type: 0,
                plane_signbits: 0,
                contents: 0,
                surface: None,
                hit: TraceHitSlot::None,
            },
            trace_queries: Vec::new(),
            point_contents_value: 0,
            in_pvs_result: true,
            in_phs_result: true,
            inline_bounds: Bounds { min: zero, max: zero },
            body_links: HashMap::new(),
            bound_entities: Vec::new(),
            linked_entities: Vec::new(),
            unlinked_entities: Vec::new(),
            numeric,
            pmove_options: ClassicGuestPmoveOptions::default(),
        }
    }

    /// Assign or reuse a resource index.
    pub fn resource_index(&mut self, kind: &str, name: &str) -> i32 {
        match self.resources.entry((kind.to_string(), name.to_string())) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let index = self.next_resource;
                self.next_resource += 1;
                *entry.insert(index)
            }
        }
    }
}

/// Live tag allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClassicAllocation {
    address: GuestAddress,
    bytes: usize,
    tag: i32,
}

/// Exclusive pickup supply lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickupSupplyLease {
    /// Lease owner.
    pub owner: String,
}

/// Client connect outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConnectOutcome {
    /// Whether the client was admitted.
    pub allowed: bool,
    /// Possibly rewritten userinfo.
    pub userinfo: String,
}

fn void_result() -> GuestCallResult {
    GuestCallResult::Void
}

fn pointer_result(value: Option<GuestAddress>) -> GuestCallResult {
    GuestCallResult::Value(GuestCallValue::Pointer(value))
}

fn int_result(value: i32) -> GuestCallResult {
    GuestCallResult::Value(GuestCallValue::Int32(value))
}

fn arg_pointer(args: &[GuestCallValue], index: usize) -> ClassicResult<Option<GuestAddress>> {
    let Some(GuestCallValue::Pointer(value)) = args.get(index) else {
        return Err(ClassicQ2Error::invalid(format!(
            "API 3 argument {index} must be a pointer"
        )));
    };
    Ok(*value)
}

fn arg_required(args: &[GuestCallValue], index: usize) -> ClassicResult<GuestAddress> {
    match arg_pointer(args, index)? {
        Some(address) => Ok(address),
        None => Err(ClassicQ2Error::invalid(format!(
            "API 3 argument {index} cannot be null"
        ))),
    }
}

fn arg_int(args: &[GuestCallValue], index: usize) -> ClassicResult<i32> {
    match args.get(index) {
        Some(GuestCallValue::Int32(value)) => Ok(*value),
        Some(GuestCallValue::Uint32(value)) => Ok(*value as i32),
        Some(GuestCallValue::Float32(value)) => Ok(float_to_wrapped_i32(f64::from(*value))),
        Some(GuestCallValue::Float64(value)) => Ok(float_to_wrapped_i32(*value)),
        _ => Err(ClassicQ2Error::invalid(format!(
            "API 3 argument {index} must be numeric"
        ))),
    }
}

fn arg_float(args: &[GuestCallValue], index: usize) -> ClassicResult<f32> {
    match args.get(index) {
        Some(GuestCallValue::Float32(value)) => Ok(*value),
        Some(GuestCallValue::Float64(value)) => Ok(*value as f32),
        Some(GuestCallValue::Int32(value)) => Ok(*value as f32),
        Some(GuestCallValue::Uint32(value)) => Ok(*value as f32),
        _ => Err(ClassicQ2Error::invalid(format!(
            "API 3 argument {index} must be numeric"
        ))),
    }
}

fn arg_string(memory: &mut SparseGuestMemory, args: &[GuestCallValue], index: usize) -> ClassicResult<String> {
    read_classic_string(memory, arg_pointer(args, index)?, 65536)
}

fn arg_vector(memory: &mut SparseGuestMemory, args: &[GuestCallValue], index: usize) -> ClassicResult<Vec3> {
    read_classic_vector(memory, arg_required(args, index)?)
}

/// The DLL owns gameplay bytes. Every call runs a registered synthetic
/// guest handler against the shared guest memory.
pub struct ClassicQ2GuestHost {
    /// Guest memory shared by every call.
    pub memory: SparseGuestMemory,
    /// Actor registry shared with the edict roster.
    pub registry: ActorRegistry,
    /// Guest cvar view.
    pub cvars: ClassicQ2Cvars,
    /// Engine services.
    pub services: ClassicEngineServices,
    /// Edict roster once `GetGameAPI` has run.
    pub edicts: Option<ClassicQ2Edicts>,
    /// Owning provider.
    pub provider: ProviderId,
    /// Import table base.
    pub imports: GuestAddress,
    /// Default instruction budget per call.
    pub instruction_budget: u64,
    /// Synthetic instructions executed so far.
    pub instructions_executed: u64,
    guest_handlers: HashMap<u64, GuestHandler>,
    allocations: HashMap<u64, ClassicAllocation>,
    surfaces: HashMap<(String, i32, i32), GuestAddress>,
    models: HashMap<i32, String>,
    exports: Option<GuestAddress>,
    initialized: bool,
    suppress_reconcile: bool,
    input_movement: Option<InputMovementBoundary>,
    pickup_profile: Option<ClassicPickupProfile>,
    pickup_supply_owner: Option<String>,
    spawn_instructions: u64,
}

impl ClassicQ2GuestHost {
    /// Create a host over fresh guest memory and a fresh registry.
    pub fn new(provider: ProviderId, module: ModuleIdentity, instruction_budget: u64) -> ClassicResult<Self> {
        let mut memory = SparseGuestMemory::new(module, 4, 0x10000)?;
        validate_value_layout(&q2_trace(), memory.pointer_bytes())?;
        let owner = qa_core::identity::IdentityOwner::create("q2-classic")
            .map_err(|error| ClassicQ2Error::invalid(format!("classic identity: {error}")))?;
        let registry = ActorRegistry::new(owner, 2048)?;
        let numeric = NumericOps::select(Q2_DONOR_PROFILE)
            .map_err(|error| ClassicQ2Error::invalid(format!("classic numeric profile: {error}")))?;
        let imports = memory.allocate(&GuestAllocationOptions::bytes(CLASSIC_Q2_IMPORT_BYTES))?;
        let trampolines = memory.allocate(&GuestAllocationOptions::bytes(classic_q2_imports().len() * 16))?;
        let space = memory.address_space();
        for index in 0..classic_q2_imports().len() {
            let entry = GuestAddress::new(space, trampolines.offset + index as u64 * 16);
            memory.write_pointer(memory.offset(imports, index as i64 * 4)?, Some(entry))?;
        }
        Ok(Self {
            memory,
            registry,
            cvars: ClassicQ2Cvars::new(ClassicCvarRegistry::new()),
            services: ClassicEngineServices::new(numeric),
            edicts: None,
            provider,
            imports,
            instruction_budget,
            instructions_executed: 0,
            guest_handlers: HashMap::new(),
            allocations: HashMap::new(),
            surfaces: HashMap::new(),
            models: HashMap::new(),
            exports: None,
            initialized: false,
            suppress_reconcile: false,
            input_movement: None,
            pickup_profile: None,
            pickup_supply_owner: None,
            spawn_instructions: 0,
        })
    }

    /// Whether `Init` has completed.
    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Instructions executed by the last spawn.
    #[must_use]
    pub fn spawn_instructions(&self) -> u64 {
        self.spawn_instructions
    }

    /// Register a synthetic guest function at an image offset.
    pub fn register_guest_handler(&mut self, offset: u64, handler: GuestHandler) {
        self.guest_handlers.insert(offset, handler);
    }

    /// Invoke a synthetic guest function.
    pub fn invoke(
        &mut self,
        target: GuestAddress,
        signature: &GuestCallSignature,
        args: &[GuestCallValue],
        budget: u64,
    ) -> ClassicResult<GuestCallResult> {
        if budget == 0 {
            return Err(ClassicQ2Error::invalid("API 3 instruction budget exhausted"));
        }
        if !signature.variadic && args.len() != signature.parameters.len() {
            return Err(ClassicQ2Error::invalid(
                "API 3 invocation differs from its declared signature",
            ));
        }
        self.instructions_executed += 1 + args.len() as u64;
        let offset = target.offset;
        let handler = self
            .guest_handlers
            .get_mut(&target.offset)
            .ok_or_else(|| ClassicQ2Error::invalid(format!("no synthetic guest function at 0x{offset:x}")))?;
        handler(&mut self.memory, args)
    }

    /// Bind the export table returned by `GetGameAPI`.
    pub fn get_game_api(&mut self, target: GuestAddress) -> ClassicResult<GuestAddress> {
        if self.exports.is_some() {
            return Err(ClassicQ2Error::invalid("GetGameAPI already bound"));
        }
        let signature = classic_signature(vec![q2_pointer()], Some(q2_pointer()), false);
        let result = self.invoke(
            target,
            &signature,
            &[GuestCallValue::Pointer(Some(self.imports))],
            self.instruction_budget,
        )?;
        let GuestCallResult::Value(GuestCallValue::Pointer(Some(exports))) = result else {
            return Err(ClassicQ2Error::invalid(
                "GetGameAPI returned null or a non-pointer result",
            ));
        };
        let mut edicts = ClassicQ2Edicts::new(&mut self.memory, exports, self.provider.clone(), None)?;
        for entry in super::layout::classic_q2_exports() {
            let address = self
                .memory
                .read_pointer(self.memory.offset(exports, i64::from(entry.offset))?)?;
            let Some(address) = address else {
                let at = entry.offset;
                return Err(ClassicQ2Error::invalid(format!("Null API 3 export at byte {at}")));
            };
            self.memory.check(address, 1, GuestAccess::Execute)?;
        }
        edicts.descriptor(&mut self.memory)?;
        self.exports = Some(exports);
        self.edicts = Some(edicts);
        Ok(exports)
    }

    /// Call one game export by name.
    pub fn call(&mut self, name: &str, args: &[GuestCallValue]) -> ClassicResult<GuestCallResult> {
        self.call_with_budget(name, args, self.instruction_budget)
    }

    fn call_with_budget(&mut self, name: &str, args: &[GuestCallValue], budget: u64) -> ClassicResult<GuestCallResult> {
        let exports = self
            .exports
            .ok_or_else(|| ClassicQ2Error::invalid("GetGameAPI must run before lifecycle calls"))?;
        let entry =
            classic_q2_export(name).ok_or_else(|| ClassicQ2Error::invalid(format!("Unknown API 3 export {name}")))?;
        let target = self
            .memory
            .read_pointer(self.memory.offset(exports, i64::from(entry.offset))?)?
            .ok_or_else(|| ClassicQ2Error::invalid(format!("Null API 3 export {name}")))?;
        self.cvars.refresh(&mut self.memory)?;
        let result = self.invoke(target, &entry.signature, args, budget)?;
        if self.initialized && !self.suppress_reconcile && name != "Shutdown" {
            self.reconcile()?;
        }
        Ok(result)
    }

    /// Reconcile the roster, logging fresh binds.
    pub fn reconcile(&mut self) -> ClassicResult<()> {
        let edicts = self
            .edicts
            .as_mut()
            .ok_or_else(|| ClassicQ2Error::invalid("GetGameAPI has not returned its export table"))?;
        for actor in edicts.reconcile(&mut self.memory, &mut self.registry)? {
            if let Some((_, slot)) = self.registry.source_of(actor.id()) {
                self.services.bound_entities.push(slot);
            }
        }
        Ok(())
    }

    /// Observe one record, logging fresh binds.
    pub fn observe_edict(&mut self, address: GuestAddress) -> ClassicResult<Option<OwnedActor>> {
        let edicts = self
            .edicts
            .as_mut()
            .ok_or_else(|| ClassicQ2Error::invalid("GetGameAPI has not returned its export table"))?;
        match edicts.observe(&mut self.memory, &mut self.registry, address)? {
            Some(observation) => {
                let actor = observation.actor().clone();
                if matches!(observation, super::records::EdictObservation::Bound(_)) {
                    if let Some((_, slot)) = self.registry.source_of(actor.id()) {
                        self.services.bound_entities.push(slot);
                    }
                }
                Ok(Some(actor))
            }
            None => Ok(None),
        }
    }

    /// Run `Init` once.
    pub fn init(&mut self) -> ClassicResult<()> {
        if self.initialized {
            return Err(ClassicQ2Error::invalid("API 3 Init already completed"));
        }
        self.call("Init", &[])?;
        self.initialized = true;
        self.reconcile()?;
        Ok(())
    }

    /// Run `Shutdown`, releasing owned actors.
    pub fn shutdown(&mut self) -> ClassicResult<()> {
        let result = self.call("Shutdown", &[]);
        self.pickup_profile = None;
        self.pickup_supply_owner = None;
        self.initialized = false;
        result?;
        for actor in self.registry.owned_by(&self.provider) {
            self.registry.release(&actor)?;
        }
        Ok(())
    }

    /// Run `SpawnEntities` with a tenfold budget, tracking instructions.
    pub fn spawn_entities(&mut self, map: &str, entities: &str, spawn_point: &str) -> ClassicResult<()> {
        for actor in self.registry.owned_by(&self.provider) {
            self.registry.release(&actor)?;
        }
        let before = self.instructions_executed;
        let result = self.with_strings(&[map, entities, spawn_point], |host, pointers| {
            host.call_with_budget("SpawnEntities", &pointers, host.instruction_budget.saturating_mul(10))?;
            Ok(())
        });
        if result.is_ok() {
            self.spawn_instructions = self.instructions_executed - before;
        }
        result
    }

    /// Run one save export.
    pub fn save(&mut self, name: &str, filename: &str, autosave: bool) -> ClassicResult<()> {
        if !matches!(name, "WriteGame" | "ReadGame" | "WriteLevel" | "ReadLevel") {
            return Err(ClassicQ2Error::invalid(format!("Unknown API 3 save export {name}")));
        }
        self.check_pickups_idle()?;
        if matches!(name, "ReadGame" | "ReadLevel") {
            for actor in self.registry.owned_by(&self.provider) {
                self.registry.release(&actor)?;
            }
        }
        self.with_strings(&[filename], |host, pointers| {
            let mut args = pointers;
            if name == "WriteGame" {
                args.push(GuestCallValue::Int32(i32::from(autosave)));
            }
            host.call(name, &args)?;
            Ok(())
        })
    }

    fn with_strings(
        &mut self,
        texts: &[&str],
        operation: impl FnOnce(&mut Self, Vec<GuestCallValue>) -> ClassicResult<()>,
    ) -> ClassicResult<()> {
        let mut records = Vec::with_capacity(texts.len());
        for text in texts {
            let address = allocate_classic_string(&mut self.memory, text)?;
            records.push((*text, address));
        }
        let pointers: Vec<GuestCallValue> = records
            .iter()
            .map(|(_, address)| GuestCallValue::Pointer(Some(*address)))
            .collect();
        let result = operation(&mut *self, pointers);
        for (text, address) in records {
            self.memory.unmap(address, classic_string_allocation_bytes(text)?)?;
        }
        result
    }

    /// Rebind the roster after the world underneath was replaced.
    pub fn rebind_world(&mut self) -> ClassicResult<()> {
        self.check_pickups_idle()?;
        let exports = self
            .exports
            .ok_or_else(|| ClassicQ2Error::invalid("API 3 world rebind requires an idle initialized module"))?;
        if !self.initialized {
            return Err(ClassicQ2Error::invalid(
                "API 3 world rebind requires an idle initialized module",
            ));
        }
        self.edicts = Some(ClassicQ2Edicts::new(
            &mut self.memory,
            exports,
            self.provider.clone(),
            None,
        )?);
        self.models.clear();
        Ok(())
    }

    /// Rename a model index (`MAX_MODELS` bounds, empty clears).
    pub fn set_model_name(&mut self, index: i32, name: &str) -> ClassicResult<()> {
        if !(1..256).contains(&index) {
            return Err(ClassicQ2Error::invalid("API 3 model index outside MAX_MODELS"));
        }
        if name.is_empty() {
            self.models.remove(&index);
        } else {
            self.models.insert(index, name.to_string());
        }
        Ok(())
    }

    /// Save a travel level with clients temporarily marked free.
    pub fn write_travel_level(&mut self, filename: &str, max_clients: u32) -> ClassicResult<()> {
        if !self.initialized || self.suppress_reconcile {
            return Err(ClassicQ2Error::invalid(
                "API 3 travel save requires an idle initialized module",
            ));
        }
        let mut in_use = Vec::with_capacity(max_clients as usize);
        for slot in 1..=max_clients {
            let edicts = self
                .edicts
                .as_mut()
                .ok_or_else(|| ClassicQ2Error::invalid("GetGameAPI has not returned its export table"))?;
            let record = edicts.at(&mut self.memory, slot)?;
            in_use.push(self.memory.read_i32(self.memory.offset(record.address, 88)?)?);
        }
        self.suppress_reconcile = true;
        let mut failure: Option<String> = None;
        for slot in 1..=max_clients {
            let edicts = self
                .edicts
                .as_mut()
                .ok_or_else(|| ClassicQ2Error::invalid("GetGameAPI has not returned its export table"))?;
            let record = edicts.at(&mut self.memory, slot)?;
            self.memory.write_i32(self.memory.offset(record.address, 88)?, 0)?;
        }
        if let Err(error) = self.save("WriteLevel", filename, false) {
            failure = Some(error.to_string());
        }
        for (index, slot) in (1..=max_clients).enumerate() {
            let edicts = self
                .edicts
                .as_mut()
                .ok_or_else(|| ClassicQ2Error::invalid("GetGameAPI has not returned its export table"))?;
            let record = edicts.at(&mut self.memory, slot)?;
            self.memory
                .write_i32(self.memory.offset(record.address, 88)?, in_use[index])?;
        }
        self.suppress_reconcile = false;
        match (failure, self.reconcile()) {
            (Some(left), Err(right)) => Err(ClassicQ2Error::invalid(format!(
                "Travel WriteLevel and reconciliation failed: {left}; {right}"
            ))),
            (Some(left), Ok(())) => Err(ClassicQ2Error::invalid(left)),
            (None, Err(error)) => Err(error),
            (None, Ok(())) => Ok(()),
        }
    }

    /// Run one server frame.
    pub fn run_frame(&mut self) -> ClassicResult<()> {
        self.call("RunFrame", &[])?;
        Ok(())
    }

    /// Connect a client, returning admission plus possibly rewritten userinfo.
    pub fn client_connect(&mut self, slot: u32, userinfo: &str) -> ClassicResult<ClientConnectOutcome> {
        let buffer = self.memory.allocate(&GuestAllocationOptions::bytes(516))?;
        write_classic_string(&mut self.memory, buffer, userinfo, 512)?;
        let edicts = self
            .edicts
            .as_mut()
            .ok_or_else(|| ClassicQ2Error::invalid("GetGameAPI has not returned its export table"))?;
        let record = edicts.at(&mut self.memory, slot)?;
        let result = self.call(
            "ClientConnect",
            &[
                GuestCallValue::Pointer(Some(record.address)),
                GuestCallValue::Pointer(Some(buffer)),
            ],
        );
        let outcome = match result {
            Ok(GuestCallResult::Value(GuestCallValue::Int32(value))) => {
                let userinfo = read_classic_string(&mut self.memory, Some(buffer), 512)?;
                Ok(ClientConnectOutcome {
                    allowed: value != 0,
                    userinfo,
                })
            }
            Ok(_) => Err(ClassicQ2Error::invalid(
                "API 3 ClientConnect returned a non-integer result",
            )),
            Err(error) => Err(error),
        };
        self.memory.unmap(buffer, 516)?;
        outcome
    }

    /// Run a client lifecycle event.
    pub fn client_event(&mut self, name: &str, slot: u32) -> ClassicResult<()> {
        if !matches!(name, "ClientBegin" | "ClientDisconnect" | "ClientCommand") {
            return Err(ClassicQ2Error::invalid(format!("Unknown API 3 client event {name}")));
        }
        let edicts = self
            .edicts
            .as_mut()
            .ok_or_else(|| ClassicQ2Error::invalid("GetGameAPI has not returned its export table"))?;
        let record = edicts.at(&mut self.memory, slot)?;
        self.call(name, &[GuestCallValue::Pointer(Some(record.address))])?;
        Ok(())
    }

    /// Run a userinfo change, returning the possibly rewritten userinfo.
    pub fn client_userinfo_changed(&mut self, slot: u32, userinfo: &str) -> ClassicResult<String> {
        let buffer = self.memory.allocate(&GuestAllocationOptions::bytes(516))?;
        write_classic_string(&mut self.memory, buffer, userinfo, 512)?;
        let edicts = self
            .edicts
            .as_mut()
            .ok_or_else(|| ClassicQ2Error::invalid("GetGameAPI has not returned its export table"))?;
        let record = edicts.at(&mut self.memory, slot)?;
        let result = self.call(
            "ClientUserinfoChanged",
            &[
                GuestCallValue::Pointer(Some(record.address)),
                GuestCallValue::Pointer(Some(buffer)),
            ],
        );
        let outcome = result.and_then(|_| read_classic_string(&mut self.memory, Some(buffer), 512));
        self.memory.unmap(buffer, 516)?;
        outcome
    }

    /// Run a client think over 16 source command bytes.
    pub fn client_think(&mut self, slot: u32, command: &[u8]) -> ClassicResult<()> {
        if command.len() != 16 {
            return Err(ClassicQ2Error::invalid("API 3 usercmd_t requires 16 source bytes"));
        }
        let address = self.memory.allocate(&GuestAllocationOptions::bytes(16))?;
        self.memory.write(address, command)?;
        let edicts = self
            .edicts
            .as_mut()
            .ok_or_else(|| ClassicQ2Error::invalid("GetGameAPI has not returned its export table"))?;
        let record = edicts.at(&mut self.memory, slot)?;
        let result = self.call(
            "ClientThink",
            &[
                GuestCallValue::Pointer(Some(record.address)),
                GuestCallValue::Pointer(Some(address)),
            ],
        );
        self.memory.unmap(address, 16)?;
        result?;
        Ok(())
    }

    /// Variadic layouts for one import's fixed arguments.
    pub fn variadic_layouts(&mut self, name: &str, fixed: &[GuestCallValue]) -> ClassicResult<Vec<GuestValueLayout>> {
        if !classic_q2_imports().iter().any(|entry| entry.name == name) {
            return Err(ClassicQ2Error::invalid(format!(
                "Variadic callback {name} is not an API 3 import"
            )));
        }
        let index = if name == "bprintf" || name == "centerprintf" {
            1
        } else if name == "cprintf" {
            2
        } else {
            0
        };
        let format = read_classic_string(&mut self.memory, Some(arg_required(fixed, index)?), 65536)?;
        classic_printf_layouts(&format)
    }

    /// Bind the single input-movement owner.
    pub fn bind_input_movement(&mut self, boundary: InputMovementBoundary) -> ClassicResult<()> {
        if self.input_movement.is_some() {
            return Err(ClassicQ2Error::invalid("API3 movement already has an input owner"));
        }
        self.input_movement = Some(boundary);
        Ok(())
    }

    /// Release the input-movement owner.
    pub fn unbind_input_movement(&mut self) {
        self.input_movement = None;
    }

    /// Run a movement step through the input boundary, if any.
    pub fn apply_input_movement(
        &mut self,
        address: GuestAddress,
        run: &mut dyn FnMut(Option<&ClassicMovementBody>) -> ClassicResult<()>,
    ) -> ClassicResult<()> {
        match self.input_movement.take() {
            Some(mut boundary) => {
                let result = boundary(address, run);
                self.input_movement = Some(boundary);
                result
            }
            None => run(None),
        }
    }

    /// Bind primary pickups for an image, unless the digest is unadmitted.
    pub fn bind_pickups(&mut self, image: GuestAddress, declared: Option<ClassicPickupProfile>) -> ClassicResult<()> {
        if self.pickup_profile.is_some() {
            return Err(ClassicQ2Error::invalid("API3 primary pickups already bound"));
        }
        if image.space != self.memory.address_space() {
            return Err(ClassicQ2Error::invalid("Foreign pickup image address space"));
        }
        let profile = declared.or_else(|| classic_pickup_profile(&self.memory.module().digest));
        if profile.is_none() {
            return Ok(());
        }
        self.pickup_profile = profile;
        Ok(())
    }

    /// Bind the exclusive pickup supply owner.
    pub fn bind_pickup_supply(&mut self, owner: &str) -> ClassicResult<PickupSupplyLease> {
        if self.pickup_profile.is_none() {
            return Err(ClassicQ2Error::invalid(
                "This original game has no admitted pickup supply interface",
            ));
        }
        if self.pickup_supply_owner.is_some() {
            return Err(ClassicQ2Error::invalid("API3 pickup supply already has an owner"));
        }
        self.pickup_supply_owner = Some(owner.to_string());
        Ok(PickupSupplyLease {
            owner: owner.to_string(),
        })
    }

    /// Preview pickup grants against admitted entries.
    pub fn pickup_supply(
        &self,
        entries: &[InventoryEntry],
        plan: &PickupGrantPlan,
    ) -> ClassicResult<PickupSupplyPreview> {
        if self.pickup_profile.is_none() {
            return Err(ClassicQ2Error::invalid(
                "This original game has no admitted pickup supply interface",
            ));
        }
        Ok(preview_pickup_grants(entries, plan)?)
    }

    /// Release a pickup supply lease.
    pub fn release_pickup_supply(&mut self, lease: &PickupSupplyLease) -> ClassicResult<()> {
        if self.pickup_supply_owner.as_deref() != Some(lease.owner.as_str()) {
            return Err(ClassicQ2Error::invalid("API3 pickup supply lease is not current"));
        }
        self.pickup_supply_owner = None;
        Ok(())
    }

    fn check_pickups_idle(&self) -> ClassicResult<()> {
        if self.pickup_supply_owner.is_some() {
            return Err(ClassicQ2Error::invalid("API3 pickup supply is busy"));
        }
        Ok(())
    }

    /// Answer one engine import call.
    pub fn import_call(&mut self, name: &str, args: &[GuestCallValue]) -> ClassicResult<GuestCallResult> {
        match name {
            "bprintf" | "dprintf" | "cprintf" | "centerprintf" | "error" => {
                let offset = if name == "bprintf" || name == "centerprintf" {
                    1
                } else if name == "cprintf" {
                    2
                } else {
                    0
                };
                let format = arg_string(&mut self.memory, args, offset)?;
                let text = classic_printf(&mut self.memory, &format, &args[offset + 1..])?;
                if name == "error" {
                    return Err(ClassicQ2Error::invalid(format!("API 3 game error: {text}")));
                }
                let destination = match name {
                    "dprintf" => PrintDestination::Debug,
                    "bprintf" => PrintDestination::Broadcast,
                    "cprintf" => PrintDestination::Client,
                    _ => PrintDestination::Center,
                };
                let entity = if name == "cprintf" || name == "centerprintf" {
                    arg_pointer(args, 0)?
                } else {
                    None
                };
                let level = if name == "bprintf" {
                    arg_int(args, 0)?
                } else if name == "cprintf" {
                    arg_int(args, 1)?
                } else {
                    0
                };
                self.services.prints.push(PrintRecord {
                    destination,
                    entity,
                    level,
                    text,
                });
                Ok(void_result())
            }
            "TagMalloc" => {
                let requested = arg_int(args, 0)?;
                let tag = arg_int(args, 1)?;
                if requested < 0 {
                    return Err(ClassicQ2Error::invalid("TagMalloc requires a nonnegative byte count"));
                }
                let bytes = native_allocation_bytes(requested as usize)?;
                let address =
                    allocate_native_memory(&mut self.memory, requested as usize, &format!("API 3 tag {tag}"))?;
                self.allocations
                    .insert(address.offset, ClassicAllocation { address, bytes, tag });
                Ok(pointer_result(Some(address)))
            }
            "TagFree" => {
                let address = arg_required(args, 0)?;
                let record = self
                    .allocations
                    .remove(&address.offset)
                    .ok_or_else(|| ClassicQ2Error::invalid("TagFree pointer is not a live API 3 allocation"))?;
                self.memory.unmap(record.address, record.bytes)?;
                Ok(void_result())
            }
            "FreeTags" => {
                let tag = arg_int(args, 0)?;
                let doomed: Vec<u64> = self
                    .allocations
                    .iter()
                    .filter(|(_, record)| record.tag == tag)
                    .map(|(offset, _)| *offset)
                    .collect();
                for offset in doomed {
                    if let Some(record) = self.allocations.remove(&offset) {
                        self.memory.unmap(record.address, record.bytes)?;
                    }
                }
                Ok(void_result())
            }
            "cvar" => {
                let name = arg_string(&mut self.memory, args, 0)?;
                let value = arg_string(&mut self.memory, args, 1)?;
                let flags = arg_int(args, 2)?;
                self.cvars.registry_mut().register(&name, &value, flags);
                Ok(pointer_result(self.cvars.pointer(&mut self.memory, &name)?))
            }
            "cvar_set" | "cvar_forceset" => {
                let forced = name == "cvar_forceset";
                let var = arg_string(&mut self.memory, args, 0)?;
                let value = arg_string(&mut self.memory, args, 1)?;
                self.cvars.registry_mut().set(&var, &value, forced);
                Ok(pointer_result(self.cvars.pointer(&mut self.memory, &var)?))
            }
            "configstring" => {
                let index = arg_int(args, 0)?;
                let value = arg_string(&mut self.memory, args, 1)?;
                self.services.configstrings.insert(index, value);
                Ok(void_result())
            }
            "modelindex" | "soundindex" | "imageindex" => {
                let path = arg_string(&mut self.memory, args, 0)?;
                let kind = if name == "modelindex" {
                    "model"
                } else if name == "soundindex" {
                    "sound"
                } else {
                    "image"
                };
                let index = self.services.resource_index(kind, &path);
                if name == "modelindex" {
                    self.models.insert(index, path);
                }
                Ok(int_result(index))
            }
            "sound" => {
                self.services.sounds.push(SoundRecord {
                    origin: None,
                    entity: arg_pointer(args, 0)?,
                    channel: arg_int(args, 1)?,
                    index: arg_int(args, 2)?,
                    volume: arg_float(args, 3)?,
                    attenuation: arg_float(args, 4)?,
                    time_offset: arg_float(args, 5)?,
                });
                Ok(void_result())
            }
            "positioned_sound" => {
                let origin = match arg_pointer(args, 0)? {
                    Some(address) => Some(read_classic_vector(&mut self.memory, address)?),
                    None => None,
                };
                self.services.sounds.push(SoundRecord {
                    origin,
                    entity: arg_pointer(args, 1)?,
                    channel: arg_int(args, 2)?,
                    index: arg_int(args, 3)?,
                    volume: arg_float(args, 4)?,
                    attenuation: arg_float(args, 5)?,
                    time_offset: arg_float(args, 6)?,
                });
                Ok(void_result())
            }
            "pointcontents" => {
                let point = arg_vector(&mut self.memory, args, 0)?;
                let _ = point;
                Ok(int_result(self.services.point_contents_value))
            }
            "inPVS" => {
                let _start = arg_vector(&mut self.memory, args, 0)?;
                let _end = arg_vector(&mut self.memory, args, 1)?;
                Ok(int_result(i32::from(self.services.in_pvs_result)))
            }
            "inPHS" => {
                let _start = arg_vector(&mut self.memory, args, 0)?;
                let _end = arg_vector(&mut self.memory, args, 1)?;
                Ok(int_result(i32::from(self.services.in_phs_result)))
            }
            "SetAreaPortalState" => {
                self.services
                    .area_portals
                    .insert(arg_int(args, 0)?, arg_int(args, 1)? != 0);
                Ok(void_result())
            }
            "AreasConnected" => {
                let pair = (arg_int(args, 0)?, arg_int(args, 1)?);
                Ok(int_result(i32::from(
                    self.services.areas_connected.get(&pair).copied().unwrap_or(true),
                )))
            }
            "setmodel" => {
                let address = arg_required(args, 0)?;
                let edicts = self
                    .edicts
                    .as_mut()
                    .ok_or_else(|| ClassicQ2Error::invalid("GetGameAPI has not returned its export table"))?;
                let record = edicts.record_from_pointer(&mut self.memory, address)?;
                let model = arg_string(&mut self.memory, args, 1)?;
                let index = self.services.resource_index("model", &model);
                self.models.insert(index, model.clone());
                self.memory.write_i32(self.memory.offset(record.address, 40)?, index)?;
                if model.starts_with('*') {
                    model[1..]
                        .parse::<usize>()
                        .map_err(|_| ClassicQ2Error::invalid("API 3 inline model name is not numeric"))?;
                    let bounds = self.services.inline_bounds;
                    let mins_at = self.memory.offset(record.address, 188)?;
                    write_classic_vector(&mut self.memory, mins_at, bounds.min)?;
                    let maxs_at = self.memory.offset(record.address, 200)?;
                    write_classic_vector(&mut self.memory, maxs_at, bounds.max)?;
                    self.link_entity(record.address)?;
                }
                Ok(void_result())
            }
            "linkentity" => {
                let address = arg_required(args, 0)?;
                self.link_entity(address)?;
                Ok(void_result())
            }
            "unlinkentity" => {
                let address = arg_required(args, 0)?;
                if let Some(actor) = self.observe_edict(address)? {
                    if let Some((provider, slot)) = self.registry.source_of(actor.id()) {
                        if provider == self.provider {
                            if let Some(link) = self.services.body_links.get_mut(&slot) {
                                link.linked = false;
                            }
                            self.services.unlinked_entities.push(slot);
                        }
                    }
                }
                Ok(void_result())
            }
            "BoxEdicts" => {
                let mins = arg_vector(&mut self.memory, args, 0)?;
                let maxs = arg_vector(&mut self.memory, args, 1)?;
                let out = arg_required(args, 2)?;
                let maximum = arg_int(args, 3)?;
                let area = arg_int(args, 4)?;
                if maximum < 0 || (area != 1 && area != 2) {
                    return Err(ClassicQ2Error::invalid("BoxEdicts invalid area type or maximum count"));
                }
                let kind = if area == 1 { "solid" } else { "trigger" };
                self.services.box_edict_queries.push(BoxEdictsQuery {
                    bounds: Bounds { min: mins, max: maxs },
                    kind: kind.to_string(),
                    maximum,
                });
                let slots: Vec<u32> = self
                    .services
                    .box_edict_slots
                    .iter()
                    .copied()
                    .take(maximum as usize)
                    .collect();
                let mut actors = Vec::with_capacity(slots.len());
                for slot in &slots {
                    let actor = self
                        .registry
                        .at_source(&self.provider, *slot)
                        .ok_or_else(|| ClassicQ2Error::invalid(format!("BoxEdicts slot {slot} has no live actor")))?;
                    actors.push(actor);
                }
                for (index, actor) in actors.iter().enumerate() {
                    let edicts = self
                        .edicts
                        .as_mut()
                        .ok_or_else(|| ClassicQ2Error::invalid("GetGameAPI has not returned its export table"))?;
                    let address = edicts.pointer(&mut self.memory, &self.registry, actor.id())?;
                    self.memory
                        .write_pointer(self.memory.offset(out, index as i64 * 4)?, Some(address))?;
                }
                Ok(int_result(actors.len() as i32))
            }
            "trace" => {
                let start = arg_vector(&mut self.memory, args, 0)?;
                let mins = match arg_pointer(args, 1)? {
                    Some(address) => Some(read_classic_vector(&mut self.memory, address)?),
                    None => None,
                };
                let maxs = match arg_pointer(args, 2)? {
                    Some(address) => Some(read_classic_vector(&mut self.memory, address)?),
                    None => None,
                };
                let end = arg_vector(&mut self.memory, args, 3)?;
                let ignored = arg_pointer(args, 4)?;
                let mask = arg_int(args, 5)?;
                let ignore = match ignored {
                    Some(address) => match self.observe_edict(address)? {
                        Some(actor) => self.registry.source_of(actor.id()).map(|(_, slot)| slot),
                        None => None,
                    },
                    None => None,
                };
                self.services.trace_queries.push(TraceQuery {
                    start,
                    end,
                    mins,
                    maxs,
                    ignore,
                    mask,
                });
                let result = self.services.trace_result.clone();
                let bytes = self.trace_bytes(&result)?;
                Ok(GuestCallResult::Value(GuestCallValue::Aggregate {
                    layout: super::layout::classic_trace_layout(),
                    bytes,
                }))
            }
            "Pmove" => {
                let address = arg_required(args, 0)?;
                match self.input_movement.take() {
                    Some(mut boundary) => {
                        let result = boundary(address, &mut |body| self.run_pmove_with_body(address, body));
                        self.input_movement = Some(boundary);
                        result?;
                    }
                    None => self.run_pmove_with_body(address, None)?,
                }
                Ok(void_result())
            }
            "argc" => Ok(int_result(self.services.command_args.len() as i32)),
            "argv" => {
                let index = arg_int(args, 0)?;
                let value = self
                    .services
                    .command_args
                    .get(index as usize)
                    .cloned()
                    .unwrap_or_default();
                Ok(pointer_result(Some(self.cvars.string(&mut self.memory, &value)?)))
            }
            "args" => {
                let value = self.services.command_string.clone();
                Ok(pointer_result(Some(self.cvars.string(&mut self.memory, &value)?)))
            }
            "AddCommandString" => {
                let text = arg_string(&mut self.memory, args, 0)?;
                self.services.added_commands.push(text);
                Ok(void_result())
            }
            "DebugGraph" => {
                self.services
                    .debug_graphs
                    .push((arg_float(args, 0)?, arg_int(args, 1)?));
                Ok(void_result())
            }
            "multicast" | "unicast" | "WriteChar" | "WriteByte" | "WriteShort" | "WriteLong" | "WriteFloat"
            | "WriteString" | "WritePosition" | "WriteDir" | "WriteAngle" => {
                self.services.messages.push(MessageRecord {
                    operation: name.to_string(),
                    values: args.to_vec(),
                });
                Ok(void_result())
            }
            _ => Err(ClassicQ2Error::invalid(format!(
                "Unsupported API 3 engine import {name}"
            ))),
        }
    }

    /// Encode a trace answer as 56 source bytes, interning surfaces.
    pub fn trace_bytes(&mut self, trace: &ClassicTraceResult) -> ClassicResult<Vec<u8>> {
        let mut bytes = vec![0u8; 56];
        bytes[0..4].copy_from_slice(&i32::from(trace.all_solid).to_le_bytes());
        bytes[4..8].copy_from_slice(&i32::from(trace.start_solid).to_le_bytes());
        bytes[8..12].copy_from_slice(&trace.fraction.to_le_bytes());
        for (offset, vector) in [(12, trace.end), (24, trace.plane_normal)] {
            bytes[offset..offset + 4].copy_from_slice(&vector.x.to_le_bytes());
            bytes[offset + 4..offset + 8].copy_from_slice(&vector.y.to_le_bytes());
            bytes[offset + 8..offset + 12].copy_from_slice(&vector.z.to_le_bytes());
        }
        bytes[36..40].copy_from_slice(&trace.plane_dist.to_le_bytes());
        bytes[40] = trace.plane_type;
        bytes[41] = trace.plane_signbits;
        bytes[48..52].copy_from_slice(&trace.contents.to_le_bytes());
        if let Some(surface) = &trace.surface {
            let address = match self
                .surfaces
                .entry((surface.name.clone(), surface.flags, surface.value))
            {
                Entry::Occupied(entry) => *entry.get(),
                Entry::Vacant(entry) => {
                    let address = self.memory.allocate(&GuestAllocationOptions {
                        byte_length: 24,
                        alignment: 4,
                        permissions: GuestPermissions::ReadWrite,
                        label: "API 3 csurface_t".to_string(),
                    })?;
                    let mut raw = vec![0u8; 24];
                    for (index, byte) in surface.name.bytes().take(16).enumerate() {
                        raw[index] = byte;
                    }
                    raw[16..20].copy_from_slice(&surface.flags.to_le_bytes());
                    raw[20..24].copy_from_slice(&surface.value.to_le_bytes());
                    self.memory.write(address, &raw)?;
                    *entry.insert(address)
                }
            };
            bytes[44..48].copy_from_slice(&(address.offset as u32).to_le_bytes());
        }
        let entity = match trace.hit {
            TraceHitSlot::None => None,
            TraceHitSlot::World => {
                let edicts = self
                    .edicts
                    .as_mut()
                    .ok_or_else(|| ClassicQ2Error::invalid("GetGameAPI has not returned its export table"))?;
                Some(edicts.at(&mut self.memory, 0)?.address)
            }
            TraceHitSlot::Slot(slot) => {
                let actor = self
                    .registry
                    .at_source(&self.provider, slot)
                    .ok_or_else(|| ClassicQ2Error::invalid(format!("Trace hit slot {slot} has no live actor")))?;
                let edicts = self
                    .edicts
                    .as_mut()
                    .ok_or_else(|| ClassicQ2Error::invalid("GetGameAPI has not returned its export table"))?;
                Some(edicts.pointer(&mut self.memory, &self.registry, actor.id())?)
            }
        };
        bytes[52..56].copy_from_slice(&(entity.map_or(0, |address| address.offset as u32)).to_le_bytes());
        Ok(bytes)
    }

    /// Run movement over one guest `pmove_t` block.
    pub fn run_pmove(&mut self, address: GuestAddress) -> ClassicResult<()> {
        self.run_pmove_with_body(address, None)
    }

    fn run_pmove_with_body(&mut self, address: GuestAddress, body: Option<&ClassicMovementBody>) -> ClassicResult<()> {
        let edicts = self
            .edicts
            .as_mut()
            .ok_or_else(|| ClassicQ2Error::invalid("GetGameAPI has not returned its export table"))?;
        let count = edicts.descriptor(&mut self.memory)?.count;
        let mut slots = HashMap::with_capacity(count);
        for slot in 0..count as u32 {
            slots.insert(slot, edicts.at(&mut self.memory, slot)?.address);
        }
        let mut trace = HostPmoveTrace {
            result: self.services.trace_result.clone(),
            contents: self.services.point_contents_value,
            registry: &self.registry,
            provider: self.provider.clone(),
            queries: Vec::new(),
        };
        let mut entities = HostPmoveEntities {
            registry: &self.registry,
            provider: self.provider.clone(),
            slots,
        };
        let options = self.services.pmove_options.clone();
        let numeric = self.services.numeric;
        run_classic_guest_pmove(
            &mut self.memory,
            address,
            &options,
            numeric,
            body,
            &mut trace,
            &mut entities,
        )?;
        self.services.trace_queries.extend(trace.queries);
        Ok(())
    }

    /// Link one entity record into the world.
    pub fn link_entity(&mut self, address: GuestAddress) -> ClassicResult<()> {
        let edicts = self
            .edicts
            .as_mut()
            .ok_or_else(|| ClassicQ2Error::invalid("GetGameAPI has not returned its export table"))?;
        let record = edicts.record_from_pointer(&mut self.memory, address)?;
        if record.slot == 0 || edicts.observe(&mut self.memory, &mut self.registry, address)?.is_none() {
            return Ok(());
        }
        if let Some(link) = self.services.body_links.get_mut(&record.slot) {
            link.linked = false;
        }
        let origin_at = self.memory.offset(address, 4)?;
        let origin = read_classic_vector(&mut self.memory, origin_at)?;
        let angles_at = self.memory.offset(address, 16)?;
        let angles = read_classic_vector(&mut self.memory, angles_at)?;
        let mins_at = self.memory.offset(address, 188)?;
        let mins = read_classic_vector(&mut self.memory, mins_at)?;
        let maxs_at = self.memory.offset(address, 200)?;
        let maxs = read_classic_vector(&mut self.memory, maxs_at)?;
        let solid = self.memory.read_i32(self.memory.offset(address, 248)?)?;
        let flags = self.memory.read_i32(self.memory.offset(address, 184)?)?;
        if !(0..=3).contains(&solid) {
            return Err(ClassicQ2Error::invalid("Invalid API 3 solid_t"));
        }
        let clamp = |value: f32, maximum: i32| -> i32 { (value.trunc() as i32).clamp(1, maximum) };
        let encoded = if solid == 2 && (flags & 2) == 0 {
            clamp(maxs.x / 8.0, 31) | clamp(-mins.z / 8.0, 31) << 5 | clamp((maxs.z + 32.0) / 8.0, 63) << 10
        } else if solid == 3 {
            31
        } else {
            0
        };
        self.memory.write_i32(self.memory.offset(address, 72)?, encoded)?;
        let radius = if solid == 3 && (angles.x != 0.0 || angles.y != 0.0 || angles.z != 0.0) {
            Some(
                mins.x
                    .abs()
                    .max(mins.y.abs())
                    .max(mins.z.abs())
                    .max(maxs.x.abs())
                    .max(maxs.y.abs())
                    .max(maxs.z.abs()),
            )
        } else {
            None
        };
        let bounds = match radius {
            Some(radius) => Bounds {
                min: Vec3 {
                    x: (origin.x - radius) - 1.0,
                    y: (origin.y - radius) - 1.0,
                    z: (origin.z - radius) - 1.0,
                },
                max: Vec3 {
                    x: (origin.x + radius) + 1.0,
                    y: (origin.y + radius) + 1.0,
                    z: (origin.z + radius) + 1.0,
                },
            },
            None => Bounds {
                min: Vec3 {
                    x: (origin.x + mins.x) - 1.0,
                    y: (origin.y + mins.y) - 1.0,
                    z: (origin.z + mins.z) - 1.0,
                },
                max: Vec3 {
                    x: (origin.x + maxs.x) + 1.0,
                    y: (origin.y + maxs.y) + 1.0,
                    z: (origin.z + maxs.z) + 1.0,
                },
            },
        };
        let absmin_at = self.memory.offset(address, 212)?;
        write_classic_vector(&mut self.memory, absmin_at, bounds.min)?;
        let absmax_at = self.memory.offset(address, 224)?;
        write_classic_vector(&mut self.memory, absmax_at, bounds.max)?;
        let size_at = self.memory.offset(address, 236)?;
        write_classic_vector(
            &mut self.memory,
            size_at,
            Vec3 {
                x: maxs.x - mins.x,
                y: maxs.y - mins.y,
                z: maxs.z - mins.z,
            },
        )?;
        let link = self.services.world_link.clone();
        let clustered = link.clusters.as_ref().is_some_and(|clusters| clusters.len() <= 16);
        self.memory.write_i32(
            self.memory.offset(address, 104)?,
            link.clusters
                .as_ref()
                .map_or(-1, |clusters| if clustered { clusters.len() as i32 } else { -1 }),
        )?;
        self.memory
            .write_i32(self.memory.offset(address, 172)?, link.headnode)?;
        self.memory.write_i32(self.memory.offset(address, 176)?, link.areas.0)?;
        self.memory.write_i32(self.memory.offset(address, 180)?, link.areas.1)?;
        if let Some(clusters) = &link.clusters {
            for (index, cluster) in clusters.iter().take(16).enumerate() {
                self.memory
                    .write_i32(self.memory.offset(address, 108 + index as i64 * 4)?, *cluster)?;
            }
        }
        if self.memory.read_i32(self.memory.offset(address, 92)?)? == 0 {
            let old_origin_at = self.memory.offset(address, 28)?;
            write_classic_vector(&mut self.memory, old_origin_at, origin)?;
        }
        let linkcount = self.memory.read_i32(self.memory.offset(address, 92)?)?;
        self.memory.write_i32(self.memory.offset(address, 92)?, linkcount + 1)?;
        let model_index = self.memory.read_i32(self.memory.offset(address, 40)?)?;
        let model_name = self.models.get(&model_index).cloned();
        if solid == 3
            && model_name.as_ref().is_none_or(|name| {
                !name.starts_with('*') || name.len() < 2 || !name[1..].bytes().all(|byte| byte.is_ascii_digit())
            })
        {
            return Err(ClassicQ2Error::invalid(
                "API 3 solid brush has no registered inline model",
            ));
        }
        let brush_model = if solid == 3 {
            model_name.as_ref().and_then(|name| name[1..].parse::<i32>().ok())
        } else {
            None
        };
        self.services.body_links.insert(
            record.slot,
            BodyLinkRecord {
                solid,
                brush_model,
                linked: false,
            },
        );
        if solid != 0 {
            if let Some(link) = self.services.body_links.get_mut(&record.slot) {
                link.linked = true;
            }
            self.services.linked_entities.push(record.slot);
        }
        Ok(())
    }
}

struct HostPmoveTrace<'r> {
    result: ClassicTraceResult,
    contents: i32,
    registry: &'r ActorRegistry,
    provider: ProviderId,
    queries: Vec<TraceQuery>,
}

fn to_src(vector: Vec3) -> SrcVec3 {
    [f64::from(vector.x), f64::from(vector.y), f64::from(vector.z)]
}

fn to_vec3(vector: SrcVec3) -> Vec3 {
    Vec3 {
        x: vector[0] as f32,
        y: vector[1] as f32,
        z: vector[2] as f32,
    }
}

impl PmoveTrace for HostPmoveTrace<'_> {
    fn trace(
        &mut self,
        start: SrcVec3,
        mins: SrcVec3,
        maxs: SrcVec3,
        end: SrcVec3,
    ) -> qa_world::movement::q2::types::TraceT {
        use qa_world::movement::q2::types::{plane, CPlane, CSurface};
        use qa_world::movement::types::TraceHit;
        self.queries.push(TraceQuery {
            start: to_vec3(start),
            end: to_vec3(end),
            mins: Some(to_vec3(mins)),
            maxs: Some(to_vec3(maxs)),
            ignore: None,
            mask: 0,
        });
        let ent = match self.result.hit {
            TraceHitSlot::None => None,
            TraceHitSlot::World => Some(TraceHit::World { model: 0 }),
            TraceHitSlot::Slot(slot) => self
                .registry
                .at_source(&self.provider, slot)
                .map(|actor| TraceHit::Actor {
                    actor: actor.id().clone(),
                }),
        };
        qa_world::movement::q2::types::TraceT {
            allsolid: self.result.all_solid,
            startsolid: self.result.start_solid,
            fraction: f64::from(self.result.fraction),
            endpos: to_src(self.result.end),
            plane: CPlane {
                normal: to_src(self.result.plane_normal),
                dist: f64::from(self.result.plane_dist),
                plane_type: i32::from(self.result.plane_type),
                signbits: i32::from(self.result.plane_signbits),
            },
            surface: self.result.surface.as_ref().map(|surface| CSurface {
                name: surface.name.clone(),
                flags: surface.flags,
                value: surface.value,
                material: String::new(),
            }),
            contents: self.result.contents,
            ent,
            plane2: plane(),
            surface2: None,
            native: None,
        }
    }

    fn point_contents(&mut self, _point: SrcVec3) -> i32 {
        self.contents
    }
}

struct HostPmoveEntities<'r> {
    registry: &'r ActorRegistry,
    provider: ProviderId,
    slots: HashMap<u32, GuestAddress>,
}

impl PmoveEntities for HostPmoveEntities<'_> {
    fn entity(&mut self, address: Option<GuestAddress>) -> ClassicResult<Option<MovementEntity>> {
        use qa_world::movement::types::TraceHit;
        let Some(address) = address else {
            return Ok(None);
        };
        let slot = self
            .slots
            .iter()
            .find(|(_, found)| **found == address)
            .map(|(slot, _)| *slot)
            .ok_or_else(|| ClassicQ2Error::invalid("Pointer does not identify the start of an API 3 edict"))?;
        if slot == 0 {
            return Ok(Some(TraceHit::World { model: 0 }));
        }
        match self.registry.at_source(&self.provider, slot) {
            Some(actor) => Ok(Some(TraceHit::Actor {
                actor: actor.id().clone(),
            })),
            None => Err(ClassicQ2Error::invalid("Pmove callback returned an inactive edict")),
        }
    }

    fn pointer(&mut self, hit: &MovementEntity) -> ClassicResult<Option<GuestAddress>> {
        use qa_world::movement::types::TraceHit;
        match hit {
            TraceHit::None => Ok(None),
            TraceHit::World { .. } => self
                .slots
                .get(&0)
                .copied()
                .map(Some)
                .ok_or_else(|| ClassicQ2Error::invalid("Pmove world entity has no edict address")),
            TraceHit::Actor { actor } => {
                let Some((provider, slot)) = self.registry.source_of(actor) else {
                    return Err(ClassicQ2Error::invalid(
                        "Foreign actor requires an explicit native semantic edict adapter",
                    ));
                };
                if provider != self.provider {
                    return Err(ClassicQ2Error::invalid(
                        "Foreign actor requires an explicit native semantic edict adapter",
                    ));
                }
                self.slots
                    .get(&slot)
                    .copied()
                    .map(Some)
                    .ok_or_else(|| ClassicQ2Error::invalid(format!("Pmove actor slot {slot} has no edict address")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::layout::classic_q2_exports;
    use super::*;
    use qa_guest::core::contracts::{ContentDigest, GuestMapOptions};

    const CODE_BASE: u64 = 0x10000000;

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
        .unwrap()
    }

    fn map_code(host: &mut ClassicQ2GuestHost) {
        host.memory
            .map(&GuestMapOptions {
                base: CODE_BASE,
                byte_length: 0x10000,
                permissions: GuestPermissions::ReadWriteExecute,
                label: "test code".to_string(),
                bytes: None,
            })
            .unwrap();
    }

    fn code(space: u64, index: u64) -> GuestAddress {
        GuestAddress::new(space, CODE_BASE + index * 0x100)
    }

    fn bind_game(host: &mut ClassicQ2GuestHost) {
        map_code(host);
        let space = host.memory.address_space();
        let get_game_api = code(space, 0);
        host.register_guest_handler(
            get_game_api.offset,
            Box::new(move |memory, _| {
                let exports = memory.allocate(&GuestAllocationOptions::bytes(80))?;
                memory.write_i32(exports, 3)?;
                for (index, entry) in classic_q2_exports().iter().enumerate() {
                    let target = code(space, 1 + index as u64);
                    memory.write_pointer(memory.offset(exports, i64::from(entry.offset))?, Some(target))?;
                }
                let edicts = memory.allocate(&GuestAllocationOptions::bytes(896 * 8))?;
                memory.write_pointer(memory.offset(exports, 64)?, Some(edicts))?;
                memory.write_i32(memory.offset(exports, 68)?, 896)?;
                memory.write_i32(memory.offset(exports, 72)?, 8)?;
                memory.write_i32(memory.offset(exports, 76)?, 8)?;
                memory.write_i32(memory.offset(edicts, 88)?, 1)?;
                Ok(pointer_result(Some(exports)))
            }),
        );
        for index in 0..classic_q2_exports().len() as u64 {
            let name = classic_q2_exports()[index as usize].name;
            if name == "ClientConnect" {
                host.register_guest_handler(code(space, 1 + index).offset, Box::new(|_, _| Ok(int_result(1))));
            } else {
                host.register_guest_handler(code(space, 1 + index).offset, Box::new(|_, _| Ok(void_result())));
            }
        }
        host.get_game_api(get_game_api).unwrap();
    }

    #[test]
    fn lifecycle_binds_exports_and_reconciles() {
        let mut host = test_host();
        bind_game(&mut host);
        assert!(host.exports.is_some());
        host.init().unwrap();
        assert!(host.is_initialized());
        assert!(host.init().is_err());
        host.run_frame().unwrap();
        assert_eq!(host.services.bound_entities, vec![0]);
        let outcome = host.client_connect(1, "\\name\\soldier").unwrap();
        assert!(outcome.allowed);
        host.client_event("ClientBegin", 1).unwrap();
        host.client_think(1, &[0u8; 16]).unwrap();
        assert!(host.client_think(1, &[0u8; 8]).is_err());
        host.shutdown().unwrap();
        assert!(!host.is_initialized());
        assert!(host.registry.owned_by(&ProviderId::new("q2", "classic")).is_empty());
    }

    #[test]
    fn imports_answer_alloc_cvars_box_trace_and_link() {
        let mut host = test_host();
        bind_game(&mut host);
        host.init().unwrap();
        let format = allocate_classic_string(&mut host.memory, "health %d").unwrap();
        host.import_call(
            "bprintf",
            &[
                GuestCallValue::Int32(2),
                GuestCallValue::Pointer(Some(format)),
                GuestCallValue::Int32(75),
            ],
        )
        .unwrap();
        assert_eq!(host.services.prints[0].text, "health 75");
        assert_eq!(host.services.prints[0].level, 2);
        assert!(host
            .import_call("error", &[GuestCallValue::Pointer(Some(format))])
            .is_err());
        let name = allocate_classic_string(&mut host.memory, "skill").unwrap();
        let value = allocate_classic_string(&mut host.memory, "2").unwrap();
        let record = host
            .import_call(
                "cvar",
                &[
                    GuestCallValue::Pointer(Some(name)),
                    GuestCallValue::Pointer(Some(value)),
                    GuestCallValue::Int32(0),
                ],
            )
            .unwrap();
        assert!(matches!(
            record,
            GuestCallResult::Value(GuestCallValue::Pointer(Some(_)))
        ));
        let alloc = host
            .import_call("TagMalloc", &[GuestCallValue::Int32(64), GuestCallValue::Int32(7)])
            .unwrap();
        let GuestCallResult::Value(GuestCallValue::Pointer(Some(block))) = alloc else {
            panic!("TagMalloc must return a pointer");
        };
        host.import_call("TagFree", &[GuestCallValue::Pointer(Some(block))])
            .unwrap();
        assert!(host
            .import_call("TagFree", &[GuestCallValue::Pointer(Some(block))])
            .is_err());
        host.import_call("FreeTags", &[GuestCallValue::Int32(7)]).unwrap();
        let model = allocate_classic_string(&mut host.memory, "models/ammo.md2").unwrap();
        let index = host
            .import_call("modelindex", &[GuestCallValue::Pointer(Some(model))])
            .unwrap();
        assert_eq!(index, int_result(1));
        host.services.box_edict_slots = vec![0];
        let mins = allocate_classic_string(&mut host.memory, "mmmmmmmmmmmm").unwrap();
        let out = host.memory.allocate(&GuestAllocationOptions::bytes(32)).unwrap();
        write_classic_vector(
            &mut host.memory,
            mins,
            Vec3 {
                x: -8.0,
                y: -8.0,
                z: -8.0,
            },
        )
        .unwrap();
        let maxs = host.memory.offset(mins, 12).unwrap();
        let found = host
            .import_call(
                "BoxEdicts",
                &[
                    GuestCallValue::Pointer(Some(mins)),
                    GuestCallValue::Pointer(Some(maxs)),
                    GuestCallValue::Pointer(Some(out)),
                    GuestCallValue::Int32(4),
                    GuestCallValue::Int32(1),
                ],
            )
            .unwrap();
        assert_eq!(found, int_result(1));
        let end = host.memory.offset(out, 8).unwrap();
        let trace = host
            .import_call(
                "trace",
                &[
                    GuestCallValue::Pointer(Some(mins)),
                    GuestCallValue::Pointer(None),
                    GuestCallValue::Pointer(None),
                    GuestCallValue::Pointer(Some(end)),
                    GuestCallValue::Pointer(None),
                    GuestCallValue::Int32(-1),
                ],
            )
            .unwrap();
        let GuestCallResult::Value(GuestCallValue::Aggregate { bytes, .. }) = trace else {
            panic!("trace must return an aggregate");
        };
        assert_eq!(bytes.len(), 56);
        assert_eq!(host.services.trace_queries.len(), 1);
        let edicts = host.edicts.as_mut().unwrap();
        let one = edicts.at(&mut host.memory, 1).unwrap();
        host.memory
            .write_i32(host.memory.offset(one.address, 88).unwrap(), 1)
            .unwrap();
        host.memory
            .write_i32(host.memory.offset(one.address, 248).unwrap(), 2)
            .unwrap();
        host.link_entity(one.address).unwrap();
        assert_eq!(host.services.linked_entities, vec![1]);
        assert!(host.services.body_links[&1].linked);
        host.import_call("unlinkentity", &[GuestCallValue::Pointer(Some(one.address))])
            .unwrap();
        assert!(!host.services.body_links[&1].linked);
        assert_eq!(host.services.unlinked_entities, vec![1]);
    }

    #[test]
    fn travel_save_client_userinfo_and_pickups() {
        let mut host = test_host();
        bind_game(&mut host);
        host.init().unwrap();
        let changed = host.client_userinfo_changed(1, "\\name\\scout").unwrap();
        assert_eq!(changed, "\\name\\scout");
        host.write_travel_level("travel.sav", 2).unwrap();
        assert!(host.import_call("bogus", &[]).is_err());
        assert!(host.call("Bogus", &[]).is_err());
        let image = host.memory.allocate(&GuestAllocationOptions::bytes(64)).unwrap();
        host.bind_pickups(image, Some(super::super::pickup_profile::xatrix_pickup_profile()))
            .unwrap();
        assert!(host.bind_pickups(image, None).is_err());
        let entries = vec![InventoryEntry {
            item: "q2:shells".to_string(),
            count: 0.0,
            capacity: 50.0,
            count_policy: None,
        }];
        let preview = host
            .pickup_supply(
                &entries,
                &PickupGrantPlan::Ammo {
                    acceptance: qa_world::pickups::AmmoAcceptance::Positive,
                    ammo: vec![qa_world::pickups::PickupAmmoGrant {
                        item: "q2:shells".to_string(),
                        amount: 10.0,
                    }],
                    weapons: qa_world::pickups::AmmoWeapons::SharedAmmo { items: vec![] },
                },
            )
            .unwrap();
        assert!(preview.accepted);
        assert_eq!(preview.ammo[0].given, 10.0);
        let lease = host.bind_pickup_supply("primary").unwrap();
        assert!(host.bind_pickup_supply("other").is_err());
        assert!(host.save("WriteGame", "game.sav", false).is_err());
        host.release_pickup_supply(&lease).unwrap();
        assert!(host.release_pickup_supply(&lease).is_err());
        host.set_model_name(3, "*4").unwrap();
        assert!(host.set_model_name(300, "x").is_err());
        let layouts = host
            .variadic_layouts("bprintf", &[GuestCallValue::Int32(0), GuestCallValue::Pointer(None)])
            .unwrap();
        assert_eq!(layouts, vec![]);
    }
}
