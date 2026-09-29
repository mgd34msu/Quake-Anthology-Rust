//! Q2 rerelease guest host: engine binding and import dispatch.
//!
//! Donor: `src/compat/q2/rerelease/host.ts` — bridges source edicts into
//! shared actor authorities and serves the native import table.

use std::collections::{HashMap, HashSet, VecDeque};

use qa_core::math::{Bounds, Vec3};
use qa_guest::core::contracts::{
    GuestAccess, GuestAddress, GuestAllocationOptions, GuestCallResult, GuestCallValue, GuestLayout,
};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::GuestError;
use thiserror::Error;

use super::layouts::{edict_layout, field_offset, surface_layout, trace_layout};

/// Host failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HostError {
    /// Primary pickup callers cannot bind a component projection.
    #[error("Primary pickup callers cannot bind a component projection")]
    ProjectionConflict,
    /// Headless Q2 debug drawing cannot bind renderer callbacks.
    #[error("Headless Q2 debug drawing cannot bind renderer callbacks")]
    HeadlessConflict,
    /// Foreign native damage requires verified entry points.
    #[error("Foreign native damage requires verified entry points")]
    ForeignWithoutEntries,
    /// Q2 BoxEdicts requires AREA_SOLID or AREA_TRIGGERS.
    #[error("Q2 BoxEdicts requires AREA_SOLID or AREA_TRIGGERS")]
    BadBoxArea,
    /// Invalid Q2 BoxEdicts capacity.
    #[error("Invalid Q2 BoxEdicts capacity")]
    BadBoxCapacity,
    /// Q2 BoxEdicts output is null with nonzero capacity.
    #[error("Q2 BoxEdicts output is null with nonzero capacity")]
    NullBoxOutput,
    /// Q2 BoxEdicts filter must return its source enum.
    #[error("Q2 BoxEdicts filter must return its source enum")]
    BadBoxFilter,
    /// Invalid Q2 BoxEdicts filter result.
    #[error("Invalid Q2 BoxEdicts filter result")]
    BadFilterResult,
    /// Q2 trace requires both bounds or neither.
    #[error("Q2 trace requires both bounds or neither")]
    SplitBounds,
    /// Q2 BoxEdicts filter cannot modify world links.
    #[error("Q2 BoxEdicts filter cannot modify world links")]
    FilterLink,
    /// Unsupported source Q2 solid.
    #[error("Unsupported source Q2 solid")]
    BadSolid,
    /// Shared body failed to retain source link.
    #[error("Shared body failed to retain source link")]
    LinkLost,
    /// Invalid source inline model name.
    #[error("Invalid source inline model name")]
    BadInlineModel,
    /// Q2 bot registration requires a live source edict.
    #[error("Q2 bot registration requires a live source edict")]
    BotWithoutEdict,
    /// Invalid Q2 client reservation.
    #[error("Invalid Q2 client reservation")]
    BadReservation,
    /// Q2 reserved source client has no actor.
    #[error("Q2 reserved source client has no actor")]
    ReservationWithoutActor,
    /// Q2 world rebind requires an idle initialized host.
    #[error("Q2 world rebind requires an idle initialized host")]
    BadRebind,
    /// Q2 guest host is closed.
    #[error("Q2 guest host is closed")]
    Closed,
    /// Q2 JSON save contains an embedded terminator.
    #[error("Q2 JSON save contains an embedded terminator")]
    SaveTerminator,
    /// Native save requires its foreign actor binding.
    #[error("Native save requires its foreign actor binding")]
    SaveWithoutForeign,
    /// Q2 source save returned null.
    #[error("Q2 source save returned null")]
    NullSave,
    /// Rerelease trace requires Q2 source collision fields.
    #[error("Rerelease trace requires Q2 source collision fields")]
    NonQ2Trace,
    /// Missing Q2 rerelease import.
    #[error("Missing Q2 rerelease {0} import {1}")]
    MissingImport(String, String),
    /// Q2 guest raised a fatal error.
    #[error("Q2 fatal: {0}")]
    GuestFatal(String),
    /// Edict slot outside capacity.
    #[error("Q2 edict slot is outside source capacity")]
    SlotOutOfRange,
    /// Actor has no foreign projection address.
    #[error("Actor has no foreign projection address")]
    NoForeignProjection,
    /// Guest memory failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

/// Engine body record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EngineBody {
    /// Origin.
    pub origin: Vec3,
    /// Bounds.
    pub bounds: Bounds,
    /// Linked flag.
    pub linked: bool,
    /// Absolute bounds when linked.
    pub absolute: Option<Bounds>,
}

/// Trace surface.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TraceSurface {
    /// Surface name.
    pub name: String,
    /// Flags.
    pub flags: u32,
    /// Value.
    pub value: i32,
    /// Material.
    pub material: String,
}

/// Trace hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceHit {
    /// World edict.
    World,
    /// Actor key.
    Actor(u32),
}

/// Q2 trace result.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2Trace {
    /// All solid.
    pub all_solid: bool,
    /// Start solid.
    pub start_solid: bool,
    /// Fraction.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Plane normal.
    pub plane_normal: Vec3,
    /// Plane distance.
    pub plane_distance: f32,
    /// Plane type.
    pub plane_type: u8,
    /// Plane sign bits.
    pub plane_signbits: u8,
    /// Surface, if any.
    pub surface: Option<TraceSurface>,
    /// Contents.
    pub contents: u32,
    /// Hit.
    pub hit: TraceHit,
    /// Secondary plane for older clients.
    pub secondary: Option<(Vec3, f32, u8, u8, Option<TraceSurface>)>,
}

/// Trace query from the native import.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceQuery {
    /// Start.
    pub start: Vec3,
    /// End.
    pub end: Vec3,
    /// Bounds, if swept.
    pub bounds: Option<Bounds>,
    /// Ignored actor.
    pub ignore: Option<u32>,
    /// Contents mask.
    pub mask: u32,
}

/// Synthetic engine behind the host.
#[derive(Debug, Default)]
pub struct SyntheticEngine {
    /// Live actors by key.
    pub live: HashMap<u32, bool>,
    /// Bodies by actor.
    pub bodies: HashMap<u32, EngineBody>,
    /// Health by actor.
    pub health: HashMap<u32, i32>,
    /// Configstrings.
    pub configstrings: HashMap<i32, String>,
    /// Cvars.
    pub cvars: HashMap<String, String>,
    /// Resource index counters.
    pub resources: HashMap<(String, String), i32>,
    /// Scripted traces.
    pub traces: VecDeque<Q2Trace>,
    /// Trace queries received.
    pub trace_queries: Vec<TraceQuery>,
    /// Point contents answer.
    pub point_contents: u32,
    /// Area portal states.
    pub portals: HashMap<i32, bool>,
    /// Areas-connected answer.
    pub areas_connected: bool,
    /// Visibility answer.
    pub visibility: bool,
    /// BoxEdicts candidates.
    pub box_edicts: Vec<u32>,
    /// Inline model bounds.
    pub inline_bounds: HashMap<i32, Bounds>,
    /// Inline model remap.
    pub inline_models: HashMap<i32, i32>,
    /// Surface ids.
    pub surface_ids: HashMap<TraceSurface, u32>,
    /// Server frame.
    pub server_frame: u32,
    /// Command arguments.
    pub command_args: Vec<String>,
    /// Printed lines.
    pub printed: Vec<String>,
    /// Queued commands.
    pub commands: Vec<String>,
    /// Link metadata answers.
    pub link_meta: HashMap<u32, (i32, i32, u32)>,
    /// Solid assignments.
    pub solids: HashMap<u32, String>,
}

impl SyntheticEngine {
    /// Surface id assignment.
    pub fn surface_id(&mut self, surface: &TraceSurface) -> u32 {
        if let Some(id) = self.surface_ids.get(surface) {
            return *id;
        }
        let id = self.surface_ids.len() as u32 + 1;
        self.surface_ids.insert(surface.clone(), id);
        id
    }
}

/// Recorded import event.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportEvent {
    /// API.
    pub api: String,
    /// Name.
    pub name: String,
    /// Argument count.
    pub arguments: usize,
}

/// Source save bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSave {
    /// Native JSON bytes.
    pub native: Vec<u8>,
    /// Deferred damage saves.
    pub deferred: Vec<DeferredSave>,
    /// Projection saves.
    pub projections: Vec<ProjectionSave>,
}

/// Deferred damage save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeferredSave {
    /// Target slot.
    pub target_slot: u32,
    /// Blood.
    pub blood: i32,
}

/// Projection save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectionSave {
    /// Native slot.
    pub slot: u32,
    /// Actor key.
    pub actor: u32,
}

/// Host construction options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostOptions {
    /// Admit primary pickups.
    pub pickups_admitted: bool,
    /// Borrow identities through a component projection.
    pub component_projection: bool,
    /// Headless debug drawing (swallow `Draw_*`).
    pub headless_debug: bool,
    /// Renderer debug-shape callback bound.
    pub debug_shapes_bound: bool,
    /// Renderer world-text callback bound.
    pub world_text_bound: bool,
    /// Foreign damage enabled.
    pub foreign_damage: bool,
    /// Native entries verified.
    pub native_entries: bool,
}

/// Source bytes back the shared actor authorities. This class owns no
/// simulation clock. Headless port over a synthetic engine.
pub struct RereleaseQ2GuestHost {
    /// Guest memory.
    pub memory: SparseGuestMemory,
    /// Entity table base.
    pub entity_base: GuestAddress,
    /// Entity stride.
    pub entity_stride: usize,
    /// Entity count.
    pub entity_count: u32,
    /// Entity capacity.
    pub entity_capacity: u32,
    /// Synthetic engine.
    pub engine: SyntheticEngine,
    /// Recorded import events.
    pub events: Vec<ImportEvent>,
    /// Recorded sounds.
    pub sounds: Vec<String>,
    /// Recorded debug shapes.
    pub shapes: Vec<String>,
    /// Recorded world texts.
    pub texts: Vec<String>,
    /// Recorded messages.
    pub messages: Vec<Vec<u8>>,
    /// Message assembly buffer.
    pub message_buffer: Vec<u8>,
    lifetimes: HashMap<u32, Lifetime>,
    retired: HashSet<u32>,
    reserved: HashSet<u32>,
    bot_entities: HashMap<u32, u32>,
    surfaces: HashMap<TraceSurface, GuestAddress>,
    foreign: HashMap<u32, GuestAddress>,
    generations: HashMap<u32, i32>,
    next_actor: u32,
    filter_depth: u32,
    initialized: bool,
    closed: bool,
    headless_debug: bool,
    foreign_enabled: bool,
    intercept: Option<Box<dyn FnMut(&str, &str, &[GuestCallValue]) -> Option<GuestCallResult>>>,
    box_filter: Option<Box<dyn FnMut(u32) -> i32>>,
    scripted_saves: VecDeque<Vec<u8>>,
    edict: GuestLayout,
    trace: GuestLayout,
    surface: GuestLayout,
}

struct Lifetime {
    actor: u32,
    generation: Option<i32>,
    address: u64,
}

impl RereleaseQ2GuestHost {
    /// Create a host over guest memory.
    pub fn new(memory: SparseGuestMemory, options: &HostOptions) -> Result<Self, HostError> {
        if options.pickups_admitted && options.component_projection {
            return Err(HostError::ProjectionConflict);
        }
        if options.headless_debug && (options.debug_shapes_bound || options.world_text_bound) {
            return Err(HostError::HeadlessConflict);
        }
        if options.foreign_damage && !options.native_entries {
            return Err(HostError::ForeignWithoutEntries);
        }
        let edict = edict_layout();
        let trace = trace_layout();
        let surface = surface_layout();
        let stride = edict.byte_length;
        let capacity = 1024u32;
        let mut memory = memory;
        let entity_base = memory.allocate(&GuestAllocationOptions::bytes(stride * capacity as usize))?;
        Ok(Self {
            memory,
            entity_base,
            entity_stride: stride,
            entity_count: 1,
            entity_capacity: capacity,
            engine: SyntheticEngine::default(),
            events: Vec::new(),
            sounds: Vec::new(),
            shapes: Vec::new(),
            texts: Vec::new(),
            messages: Vec::new(),
            message_buffer: Vec::new(),
            lifetimes: HashMap::new(),
            retired: HashSet::new(),
            reserved: HashSet::new(),
            bot_entities: HashMap::new(),
            surfaces: HashMap::new(),
            foreign: HashMap::new(),
            generations: HashMap::new(),
            next_actor: 1,
            filter_depth: 0,
            initialized: false,
            closed: false,
            headless_debug: options.headless_debug,
            foreign_enabled: options.foreign_damage && options.native_entries,
            intercept: None,
            box_filter: None,
            scripted_saves: VecDeque::new(),
            edict,
            trace,
            surface,
        })
    }

    /// Install an import interceptor.
    pub fn set_intercept(
        &mut self,
        intercept: impl FnMut(&str, &str, &[GuestCallValue]) -> Option<GuestCallResult> + 'static,
    ) {
        self.intercept = Some(Box::new(intercept));
    }

    /// Install a BoxEdicts filter callback.
    pub fn set_box_filter(&mut self, filter: impl FnMut(u32) -> i32 + 'static) {
        self.box_filter = Some(Box::new(filter));
    }

    /// Set the entity count.
    pub fn set_entity_count(&mut self, count: u32) {
        self.entity_count = count.min(self.entity_capacity);
    }

    /// Record address for a slot.
    pub fn record_at(&self, slot: u32) -> Result<GuestAddress, HostError> {
        if slot >= self.entity_capacity {
            return Err(HostError::SlotOutOfRange);
        }
        Ok(self
            .memory
            .offset(self.entity_base, slot as i64 * self.entity_stride as i64)?)
    }

    fn field(&self, name: &str) -> Result<i64, HostError> {
        field_offset(&self.edict, name)
            .map(|offset| offset as i64)
            .map_err(|_| HostError::MissingImport("game".to_string(), name.to_string()))
    }

    fn read_inuse(&mut self, slot: u32) -> Result<bool, HostError> {
        let record = self.record_at(slot)?;
        let offset = self.field("inuse")?;
        Ok(self.memory.read_u8(self.memory.offset(record, offset)?)? != 0)
    }

    /// Set the in-use flag for a slot.
    pub fn set_inuse(&mut self, slot: u32, inuse: bool) -> Result<(), HostError> {
        let record = self.record_at(slot)?;
        let offset = self.field("inuse")?;
        self.memory
            .write_u8(self.memory.offset(record, offset)?, u8::from(inuse))?;
        Ok(())
    }

    /// Set the semantic generation for a slot.
    pub fn set_generation(&mut self, slot: u32, generation: i32) {
        self.generations.insert(slot, generation);
    }

    /// Resolve the live actor for a slot, binding on first sight.
    pub fn actor(&mut self, slot: u32) -> Result<Option<u32>, HostError> {
        if self.retired.contains(&slot) {
            return Ok(None);
        }
        let live = self.read_inuse(slot)? || self.reserved.contains(&slot);
        let generation = self.generations.get(&slot).copied();
        let address = self.record_at(slot)?.offset;
        if let Some(previous) = self.lifetimes.get(&slot) {
            let stale = !live
                || previous.generation != generation
                || previous.address != address
                || !self.engine.live.get(&previous.actor).copied().unwrap_or(false);
            if stale {
                let actor = previous.actor;
                self.lifetimes.remove(&slot);
                self.engine.live.insert(actor, false);
            }
        }
        if !live {
            return Ok(None);
        }
        if let Some(current) = self.lifetimes.get(&slot) {
            return Ok(Some(current.actor));
        }
        let actor = self.next_actor;
        self.next_actor += 1;
        self.engine.live.insert(actor, true);
        self.engine.bodies.insert(
            actor,
            EngineBody {
                origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                bounds: Bounds {
                    min: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    max: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                },
                linked: false,
                absolute: None,
            },
        );
        self.engine.health.insert(actor, 100);
        self.lifetimes.insert(
            slot,
            Lifetime {
                actor,
                generation,
                address,
            },
        );
        Ok(Some(actor))
    }

    /// Retire an input client slot.
    pub fn retire_input_client(&mut self, slot: u32) {
        self.retired.insert(slot);
    }

    /// Finish input retirement for a slot.
    pub fn finish_input_retirement(&mut self, slot: u32) {
        self.retired.remove(&slot);
        self.lifetimes.remove(&slot);
    }

    /// Reserve a client slot.
    pub fn reserve_client(&mut self, slot: u32) -> Result<u32, HostError> {
        if self.closed || slot < 1 {
            return Err(HostError::BadReservation);
        }
        self.reserved.insert(slot);
        self.actor(slot)?.ok_or(HostError::ReservationWithoutActor)
    }

    /// Whether a client slot is reserved.
    #[must_use]
    pub fn is_client_reserved(&self, slot: u32) -> bool {
        self.reserved.contains(&slot)
    }

    /// Release a client reservation.
    pub fn release_client_reservation(&mut self, slot: u32) -> Result<(), HostError> {
        self.reserved.remove(&slot);
        self.reconcile()
    }

    /// Reconcile lifetimes against the entity table.
    pub fn reconcile(&mut self) -> Result<(), HostError> {
        let stale: Vec<u32> = self
            .lifetimes
            .keys()
            .copied()
            .filter(|slot| *slot >= self.entity_count)
            .collect();
        for slot in stale {
            if let Some(entry) = self.lifetimes.remove(&slot) {
                self.engine.live.insert(entry.actor, false);
            }
        }
        for slot in 0..self.entity_count {
            self.actor(slot)?;
        }
        Ok(())
    }

    /// Bot-registered entity slots.
    #[must_use]
    pub fn bot_entities(&self) -> Vec<u32> {
        self.bot_entities.values().copied().collect()
    }

    /// Guest address for an actor, projecting foreigners.
    pub fn address_for_actor(&mut self, actor: u32) -> Result<GuestAddress, HostError> {
        for (slot, lifetime) in &self.lifetimes {
            if lifetime.actor == actor {
                return self.record_at(*slot);
            }
        }
        if let Some(address) = self.foreign.get(&actor) {
            return Ok(*address);
        }
        if !self.foreign_enabled {
            return Err(HostError::NoForeignProjection);
        }
        let address = self
            .memory
            .allocate(&GuestAllocationOptions::bytes(self.entity_stride))?;
        self.foreign.insert(actor, address);
        Ok(address)
    }

    /// Resolve an actor from any known guest address.
    pub fn actor_for_address(&mut self, address: GuestAddress) -> Result<Option<u32>, HostError> {
        if let Some(actor) = self
            .foreign
            .iter()
            .find(|(_, at)| **at == address)
            .map(|(actor, _)| *actor)
        {
            return Ok(Some(actor));
        }
        Ok(self.actor(self.slot_for_address(address)?)?)
    }

    /// Slot for a guest address.
    pub fn slot_for_address(&self, address: GuestAddress) -> Result<u32, HostError> {
        if address.offset < self.entity_base.offset {
            return Err(HostError::SlotOutOfRange);
        }
        let delta = address.offset - self.entity_base.offset;
        if delta % self.entity_stride as u64 != 0
            || delta / self.entity_stride as u64 >= u64::from(self.entity_capacity)
        {
            return Err(HostError::SlotOutOfRange);
        }
        Ok((delta / self.entity_stride as u64) as u32)
    }

    /// Initialize the host.
    pub fn init(&mut self) -> Result<(), HostError> {
        if self.closed {
            return Err(HostError::Closed);
        }
        self.initialized = true;
        self.reconcile()
    }

    /// Shut down the host.
    pub fn shutdown(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        for lifetime in self.lifetimes.values() {
            self.engine.live.insert(lifetime.actor, false);
        }
        self.lifetimes.clear();
    }

    /// Rebind the world around a commit.
    pub fn rebind_world(&mut self, commit: impl FnOnce()) -> Result<(), HostError> {
        if self.closed || !self.initialized || self.filter_depth != 0 {
            return Err(HostError::BadRebind);
        }
        self.foreign.clear();
        self.reserved.clear();
        for lifetime in self.lifetimes.values() {
            self.engine.live.insert(lifetime.actor, false);
        }
        self.lifetimes.clear();
        self.bot_entities.clear();
        self.surfaces.clear();
        commit();
        Ok(())
    }

    /// Queue a scripted save payload.
    pub fn script_save(&mut self, bytes: Vec<u8>) {
        self.scripted_saves.push_back(bytes);
    }

    /// Write a save bundle; serialization runs in the guest in the donor
    /// and is scripted here.
    pub fn write_save(&mut self, kind: &str, _automatic: bool) -> Result<SourceSave, HostError> {
        let native = self.scripted_saves.pop_front().ok_or(HostError::NullSave)?;
        Ok(SourceSave {
            native,
            deferred: Vec::new(),
            projections: if kind == "level" {
                self.foreign
                    .iter()
                    .map(|(actor, _)| ProjectionSave { slot: 0, actor: *actor })
                    .collect()
            } else {
                vec![]
            },
        })
    }

    /// Read a save bundle.
    pub fn read_save(&mut self, saved: &SourceSave) -> Result<(), HostError> {
        if saved.native.contains(&0) {
            return Err(HostError::SaveTerminator);
        }
        if !self.foreign_enabled && (!saved.deferred.is_empty() || !saved.projections.is_empty()) {
            return Err(HostError::SaveWithoutForeign);
        }
        self.reconcile()
    }

    fn pointer_arg(args: &[GuestCallValue], index: usize) -> Option<GuestAddress> {
        match args.get(index) {
            Some(GuestCallValue::Pointer(address)) => *address,
            _ => None,
        }
    }

    fn int_arg(args: &[GuestCallValue], index: usize) -> i64 {
        match args.get(index) {
            Some(GuestCallValue::Int32(value)) => i64::from(*value),
            Some(GuestCallValue::Uint32(value)) => i64::from(*value),
            Some(GuestCallValue::Int64(value)) => *value,
            Some(GuestCallValue::Uint64(value)) => *value as i64,
            _ => 0,
        }
    }

    fn read_vec(&mut self, address: GuestAddress) -> Result<Vec3, HostError> {
        Ok(self.memory.read_f32x3(address)?)
    }

    fn write_vec(&mut self, address: GuestAddress, value: Vec3) -> Result<(), HostError> {
        self.memory.write_f32(address, value.x)?;
        self.memory.write_f32(self.memory.offset(address, 4)?, value.y)?;
        self.memory.write_f32(self.memory.offset(address, 8)?, value.z)?;
        Ok(())
    }

    /// Dispatch one import call.
    pub fn dispatch(&mut self, api: &str, name: &str, args: &[GuestCallValue]) -> Result<GuestCallResult, HostError> {
        self.events.push(ImportEvent {
            api: api.to_string(),
            name: name.to_string(),
            arguments: args.len(),
        });
        if let Some(intercept) = self.intercept.as_mut() {
            if let Some(result) = intercept(api, name, args) {
                return Ok(result);
            }
        }
        if self.headless_debug && api == "game" {
            match name {
                "Draw_Line"
                | "Draw_Point"
                | "Draw_Circle"
                | "Draw_Bounds"
                | "Draw_Sphere"
                | "Draw_Cylinder"
                | "Draw_Ray"
                | "Draw_Arrow"
                | "Draw_OrientedWorldText"
                | "Draw_StaticWorldText" => return Ok(GuestCallResult::Void),
                _ => {}
            }
        }
        match name {
            sound @ ("sound" | "positioned_sound" | "local_sound") if api == "game" => {
                self.sounds.push((*sound).to_string());
                Ok(GuestCallResult::Void)
            }
            shape
                if api == "game"
                    && shape.starts_with("Draw_")
                    && !matches!(shape, "Draw_OrientedWorldText" | "Draw_StaticWorldText") =>
            {
                self.shapes.push(shape.to_string());
                Ok(GuestCallResult::Void)
            }
            text @ ("Draw_OrientedWorldText" | "Draw_StaticWorldText") if api == "game" => {
                self.texts.push((*text).to_string());
                Ok(GuestCallResult::Void)
            }
            writer
                if api == "game"
                    && matches!(
                        writer,
                        "WriteChar"
                            | "WriteByte"
                            | "WriteShort"
                            | "WriteLong"
                            | "WriteFloat"
                            | "WriteString"
                            | "WritePosition"
                            | "WriteDir"
                            | "WriteAngle"
                            | "WriteEntity"
                    ) =>
            {
                self.write_message(writer, args)?;
                Ok(GuestCallResult::Void)
            }
            "multicast" | "unicast" if api == "game" => {
                self.messages.push(std::mem::take(&mut self.message_buffer));
                Ok(GuestCallResult::Void)
            }
            "Bot_RegisterEdict" | "Bot_UnRegisterEdict" => self.bot_registration(name, args),
            "FreeTags" => self.free_tags(args),
            "BoxEdicts" => self.box_edicts(args),
            "pointcontents" => {
                let address = Self::pointer_arg(args, 0)
                    .ok_or_else(|| HostError::MissingImport(api.to_string(), name.to_string()))?;
                let _ = self.read_vec(address)?;
                Ok(GuestCallResult::Value(GuestCallValue::Uint32(
                    self.engine.point_contents,
                )))
            }
            "inPVS" | "inPHS" => Ok(GuestCallResult::Value(GuestCallValue::Uint32(u32::from(
                self.engine.visibility,
            )))),
            "AreasConnected" => Ok(GuestCallResult::Value(GuestCallValue::Uint32(u32::from(
                self.engine.areas_connected,
            )))),
            "SetAreaPortalState" => {
                self.engine
                    .portals
                    .insert(Self::int_arg(args, 0) as i32, Self::int_arg(args, 1) != 0);
                Ok(GuestCallResult::Void)
            }
            "trace" => self.trace_import(args),
            "linkentity" | "unlinkentity" => self.link(args, name == "linkentity"),
            "setmodel" => self.set_model(args),
            _ => self.core_import(api, name, args),
        }
    }

    fn write_message(&mut self, writer: &str, args: &[GuestCallValue]) -> Result<(), HostError> {
        match writer {
            "WriteChar" | "WriteByte" | "WriteAngle" => {
                self.message_buffer.push(Self::int_arg(args, 0) as u8);
            }
            "WriteShort" => {
                let value = Self::int_arg(args, 0) as i16;
                self.message_buffer.extend_from_slice(&value.to_le_bytes());
            }
            "WriteLong" => {
                let value = Self::int_arg(args, 0) as i32;
                self.message_buffer.extend_from_slice(&value.to_le_bytes());
            }
            "WriteFloat" => match args.first() {
                Some(GuestCallValue::Float32(value)) => {
                    self.message_buffer.extend_from_slice(&value.to_le_bytes());
                }
                _ => self.message_buffer.extend_from_slice(&0f32.to_le_bytes()),
            },
            "WritePosition" => {
                if let Some(address) = Self::pointer_arg(args, 0) {
                    let position = self.read_vec(address)?;
                    for lane in [position.x, position.y, position.z] {
                        self.message_buffer.extend_from_slice(&lane.to_le_bytes());
                    }
                }
            }
            "WriteDir" => self.message_buffer.push(0),
            "WriteString" => {
                if let Some(address) = Self::pointer_arg(args, 0) {
                    let mut length = 0;
                    while self.memory.read_u8(self.memory.offset(address, length)?)? != 0 {
                        length += 1;
                    }
                    let bytes = self.memory.copy(address, length as usize + 1)?;
                    self.message_buffer.extend_from_slice(&bytes);
                } else {
                    self.message_buffer.push(0);
                }
            }
            "WriteEntity" => {
                if let Some(address) = Self::pointer_arg(args, 0) {
                    let slot = self.slot_for_address(address)?;
                    self.message_buffer.extend_from_slice(&(slot as i16).to_le_bytes());
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn bot_registration(&mut self, name: &str, args: &[GuestCallValue]) -> Result<GuestCallResult, HostError> {
        let address =
            Self::pointer_arg(args, 0).ok_or_else(|| HostError::MissingImport("game".to_string(), name.to_string()))?;
        let slot = self.slot_for_address(address)?;
        let actor = self.actor(slot)?;
        if name == "Bot_RegisterEdict" {
            let actor = actor.ok_or(HostError::BotWithoutEdict)?;
            self.bot_entities.insert(actor, slot);
        } else if let Some(actor) = actor {
            self.bot_entities.remove(&actor);
        }
        Ok(GuestCallResult::Void)
    }

    fn free_tags(&mut self, args: &[GuestCallValue]) -> Result<GuestCallResult, HostError> {
        let tag = Self::int_arg(args, 0);
        if tag == 765 || tag == 766 {
            self.foreign.clear();
            for lifetime in self.lifetimes.values() {
                self.engine.live.insert(lifetime.actor, false);
            }
            self.lifetimes.clear();
        }
        Ok(GuestCallResult::Void)
    }

    fn box_edicts(&mut self, args: &[GuestCallValue]) -> Result<GuestCallResult, HostError> {
        let min_address = Self::pointer_arg(args, 0)
            .ok_or_else(|| HostError::MissingImport("game".to_string(), "BoxEdicts".to_string()))?;
        let max_address = Self::pointer_arg(args, 1)
            .ok_or_else(|| HostError::MissingImport("game".to_string(), "BoxEdicts".to_string()))?;
        let _ = self.read_vec(min_address)?;
        let _ = self.read_vec(max_address)?;
        let list = Self::pointer_arg(args, 2);
        let maximum = Self::int_arg(args, 3);
        let area = Self::int_arg(args, 4);
        let filter = Self::pointer_arg(args, 5);
        if area != 1 && area != 2 {
            return Err(HostError::BadBoxArea);
        }
        if maximum < 0 {
            return Err(HostError::BadBoxCapacity);
        }
        if maximum > 0 && list.is_none() {
            return Err(HostError::NullBoxOutput);
        }
        let mut count = 0i64;
        let candidates = self.engine.box_edicts.clone();
        for candidate in candidates {
            let mut outcome = 0;
            if filter.is_some() {
                self.filter_depth += 1;
                let filtered = self.box_filter.as_mut().map(|filter| filter(candidate));
                self.filter_depth -= 1;
                outcome = filtered.unwrap_or(0);
            }
            if (outcome & !65) != 0 {
                return Err(HostError::BadFilterResult);
            }
            if (outcome & 1) == 0 {
                if maximum == 0 {
                    count += 1;
                } else if count < maximum {
                    if let Some(list) = list {
                        let address = self.address_for_actor(candidate)?;
                        self.memory
                            .write_pointer(self.memory.offset(list, count * 8)?, Some(address))?;
                    }
                    count += 1;
                }
            }
            if (outcome & 64) != 0 {
                break;
            }
        }
        Ok(GuestCallResult::Value(GuestCallValue::Uint64(count as u64)))
    }

    fn trace_import(&mut self, args: &[GuestCallValue]) -> Result<GuestCallResult, HostError> {
        let start = self.read_vec(
            Self::pointer_arg(args, 0)
                .ok_or_else(|| HostError::MissingImport("game".to_string(), "trace".to_string()))?,
        )?;
        let end = self.read_vec(
            Self::pointer_arg(args, 3)
                .ok_or_else(|| HostError::MissingImport("game".to_string(), "trace".to_string()))?,
        )?;
        let min = Self::pointer_arg(args, 1);
        let max = Self::pointer_arg(args, 2);
        if min.is_none() != max.is_none() {
            return Err(HostError::SplitBounds);
        }
        let bounds = match (min, max) {
            (Some(min), Some(max)) => Some(Bounds {
                min: self.read_vec(min)?,
                max: self.read_vec(max)?,
            }),
            _ => None,
        };
        let ignore = match Self::pointer_arg(args, 4) {
            None => None,
            Some(pass) => self.actor_for_address(pass)?,
        };
        let query = TraceQuery {
            start,
            end,
            bounds,
            ignore,
            mask: Self::int_arg(args, 5) as u32,
        };
        self.engine.trace_queries.push(query);
        let trace = self.engine.traces.pop_front().ok_or(HostError::NonQ2Trace)?;
        self.encode_trace(&trace, None)
    }

    fn link(&mut self, args: &[GuestCallValue], link: bool) -> Result<GuestCallResult, HostError> {
        if self.filter_depth != 0 {
            return Err(HostError::FilterLink);
        }
        let address = Self::pointer_arg(args, 0)
            .ok_or_else(|| HostError::MissingImport("game".to_string(), "linkentity".to_string()))?;
        if self.foreign.values().any(|at| *at == address) {
            return Ok(GuestCallResult::Void);
        }
        let slot = self.slot_for_address(address)?;
        if slot == 0 {
            return Ok(GuestCallResult::Void);
        }
        let linked_field = self.field("linked")?;
        let record = self.record_at(slot)?;
        let linked_at = self.memory.offset(record, linked_field)?;
        let Some(actor) = self.actor(slot)? else {
            self.memory.write_u8(linked_at, 0)?;
            return Ok(GuestCallResult::Void);
        };
        if !link {
            if let Some(body) = self.engine.bodies.get_mut(&actor) {
                body.linked = false;
            }
            self.memory.write_u8(linked_at, 0)?;
            return Ok(GuestCallResult::Void);
        }
        let solid = self.memory.read_u8(self.memory.offset(record, self.field("solid")?)?)?;
        if solid > 3 {
            return Err(HostError::BadSolid);
        }
        self.engine.solids.insert(
            actor,
            match solid {
                0 => "none",
                1 => "trigger",
                2 => "box",
                _ => "brush",
            }
            .to_string(),
        );
        let (area, area2, network_solid) = self.engine.link_meta.get(&actor).copied().unwrap_or((0, 0, 0));
        let absolute = Bounds {
            min: self.read_vec(self.memory.offset(record, self.field("mins")?)?)?,
            max: self.read_vec(self.memory.offset(record, self.field("maxs")?)?)?,
        };
        if let Some(body) = self.engine.bodies.get_mut(&actor) {
            body.linked = true;
            body.absolute = Some(absolute);
        }
        let linked = self
            .engine
            .bodies
            .get(&actor)
            .and_then(|body| body.absolute)
            .ok_or(HostError::LinkLost)?;
        self.write_vec(self.memory.offset(record, self.field("absmin")?)?, linked.min)?;
        self.write_vec(self.memory.offset(record, self.field("absmax")?)?, linked.max)?;
        let min = self.read_vec(self.memory.offset(record, self.field("mins")?)?)?;
        let max = self.read_vec(self.memory.offset(record, self.field("maxs")?)?)?;
        self.write_vec(
            self.memory.offset(record, self.field("size")?)?,
            Vec3 {
                x: max.x - min.x,
                y: max.y - min.y,
                z: max.z - min.z,
            },
        )?;
        self.memory
            .write_i32(self.memory.offset(record, self.field("areanum")?)?, area)?;
        self.memory
            .write_i32(self.memory.offset(record, self.field("areanum2")?)?, area2)?;
        self.memory
            .write_u32(self.memory.offset(record, self.field("s.solid")?)?, network_solid)?;
        let linkcount_at = self.memory.offset(record, self.field("linkcount")?)?;
        let renderfx = self
            .memory
            .read_u32(self.memory.offset(record, self.field("s.renderfx")?)?)?;
        if self.memory.read_i32(linkcount_at)? == 0 && (renderfx & 128) == 0 {
            let origin = self.read_vec(self.memory.offset(record, self.field("s.origin")?)?)?;
            self.write_vec(self.memory.offset(record, self.field("s.old_origin")?)?, origin)?;
        }
        let count = self.memory.read_i32(linkcount_at)?;
        self.memory.write_i32(linkcount_at, count + 1)?;
        self.memory.write_u8(linked_at, u8::from(solid != 0))?;
        Ok(GuestCallResult::Void)
    }

    fn set_model(&mut self, args: &[GuestCallValue]) -> Result<GuestCallResult, HostError> {
        if self.filter_depth != 0 {
            return Err(HostError::FilterLink);
        }
        let address = Self::pointer_arg(args, 0)
            .ok_or_else(|| HostError::MissingImport("game".to_string(), "setmodel".to_string()))?;
        let name_address = Self::pointer_arg(args, 1)
            .ok_or_else(|| HostError::MissingImport("game".to_string(), "setmodel".to_string()))?;
        let slot = self.slot_for_address(address)?;
        let found = self.memory.find_zero(name_address, 1024)?;
        if found < 0 {
            return Err(HostError::BadInlineModel);
        }
        let bytes = self.memory.copy(name_address, found as usize)?;
        let name = String::from_utf8_lossy(&bytes).into_owned();
        let index = self.engine.resources.len() as i32 + 1;
        self.engine.resources.insert(("model".to_string(), name.clone()), index);
        let record = self.record_at(slot)?;
        self.memory
            .write_i32(self.memory.offset(record, self.field("s.modelindex")?)?, index)?;
        if let Some(number) = name.strip_prefix('*') {
            let model: i32 = number.parse().map_err(|_| HostError::BadInlineModel)?;
            if model < 0 {
                return Err(HostError::BadInlineModel);
            }
            let remapped = self.engine.inline_models.get(&model).copied().unwrap_or(model);
            let bounds = self.engine.inline_bounds.get(&remapped).copied().unwrap_or(Bounds {
                min: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                max: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            });
            self.write_vec(self.memory.offset(record, self.field("mins")?)?, bounds.min)?;
            self.write_vec(self.memory.offset(record, self.field("maxs")?)?, bounds.max)?;
            let relink = [GuestCallValue::Pointer(Some(address))];
            self.link(&relink, true)?;
        }
        Ok(GuestCallResult::Void)
    }

    fn core_import(&mut self, api: &str, name: &str, args: &[GuestCallValue]) -> Result<GuestCallResult, HostError> {
        let text_arg = |index: usize| {
            Self::pointer_arg(args, index)
                .map(|address| {
                    self.memory
                        .find_zero(address, 1_048_576)
                        .map_err(HostError::from)
                        .and_then(|found| {
                            if found < 0 {
                                return Err(HostError::GuestFatal("unterminated".to_string()));
                            }
                            self.memory
                                .copy(address, found as usize)
                                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                                .map_err(HostError::from)
                        })
                })
                .transpose()
        };
        match name {
            "Com_Print" => {
                if let Some(text) = text_arg(0)? {
                    self.engine.printed.push(text);
                }
                Ok(GuestCallResult::Void)
            }
            "Com_Error" => Err(HostError::GuestFatal(text_arg(0)?.unwrap_or_default())),
            "configstring" => {
                self.engine
                    .configstrings
                    .insert(Self::int_arg(args, 0) as i32, text_arg(1)?.unwrap_or_default());
                Ok(GuestCallResult::Void)
            }
            "get_configstring" => {
                let value = self
                    .engine
                    .configstrings
                    .get(&(Self::int_arg(args, 0) as i32))
                    .cloned()
                    .unwrap_or_default();
                let address = self.memory.allocate(&GuestAllocationOptions::bytes(value.len() + 1))?;
                self.memory.write(address, value.as_bytes())?;
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(address))))
            }
            "modelindex" | "soundindex" | "imageindex" => {
                let kind = name.strip_suffix("index").unwrap_or(name).to_string();
                let text = text_arg(0)?.unwrap_or_default();
                let next = self.engine.resources.len() as i32 + 1;
                let index = *self.engine.resources.entry((kind, text)).or_insert(next);
                Ok(GuestCallResult::Value(GuestCallValue::Int32(index)))
            }
            "ServerFrame" => Ok(GuestCallResult::Value(GuestCallValue::Uint32(self.engine.server_frame))),
            "GetExtension" => Ok(GuestCallResult::Value(GuestCallValue::Pointer(None))),
            "cvar" | "cvar_set" | "cvar_forceset" => {
                let key = text_arg(0)?.unwrap_or_default();
                let value = text_arg(1)?.unwrap_or_default();
                self.engine.cvars.insert(key, value);
                let address = self.memory.allocate(&GuestAllocationOptions::bytes(56))?;
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(address))))
            }
            "TagMalloc" => {
                let size = Self::int_arg(args, 0).max(1) as usize;
                let address = self.memory.allocate(&GuestAllocationOptions::bytes(size))?;
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(address))))
            }
            "TagFree" => Ok(GuestCallResult::Void),
            _ => Err(HostError::MissingImport(api.to_string(), name.to_string())),
        }
    }

    /// Encode a Q2 trace into guest `trace_t` bytes.
    pub fn encode_trace(
        &mut self,
        trace: &Q2Trace,
        clipped: Option<GuestAddress>,
    ) -> Result<GuestCallResult, HostError> {
        let layout = self.trace.clone();
        let mut bytes = vec![0u8; layout.byte_length];
        let put_u8 = |bytes: &mut [u8], offset: usize, value: u8| {
            bytes[offset] = value;
        };
        let put_f32 = |bytes: &mut [u8], offset: usize, value: f32| {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        };
        let put_u32 = |bytes: &mut [u8], offset: usize, value: u32| {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        };
        let put_u64 = |bytes: &mut [u8], offset: usize, value: u64| {
            bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        };
        let put_vec = |bytes: &mut [u8], offset: usize, value: Vec3| {
            put_f32(bytes, offset, value.x);
            put_f32(bytes, offset + 4, value.y);
            put_f32(bytes, offset + 8, value.z);
        };
        let at = |name: &str| {
            field_offset(&layout, name).map_err(|_| HostError::MissingImport("game".to_string(), name.to_string()))
        };
        put_u8(&mut bytes, at("allsolid")?, u8::from(trace.all_solid));
        put_u8(&mut bytes, at("startsolid")?, u8::from(trace.start_solid));
        put_f32(&mut bytes, at("fraction")?, trace.fraction);
        put_vec(&mut bytes, at("endpos")?, trace.end);
        put_vec(&mut bytes, at("plane.normal")?, trace.plane_normal);
        put_f32(&mut bytes, at("plane.dist")?, trace.plane_distance);
        put_u8(&mut bytes, at("plane.type")?, trace.plane_type);
        put_u8(&mut bytes, at("plane.signbits")?, trace.plane_signbits);
        let surface = match &trace.surface {
            None => 0,
            Some(info) => self.intern_surface(info)?,
        };
        put_u64(&mut bytes, at("surface")?, surface);
        put_u32(&mut bytes, at("contents")?, trace.contents);
        let hit = match clipped {
            Some(address) => address.offset,
            None => match trace.hit {
                TraceHit::Actor(actor) => self.address_for_actor(actor)?.offset,
                TraceHit::World => self.record_at(0)?.offset,
            },
        };
        put_u64(&mut bytes, at("ent")?, hit);
        if let Some((normal, distance, plane_type, signbits, surface)) = &trace.secondary {
            put_vec(&mut bytes, at("plane2.normal")?, *normal);
            put_f32(&mut bytes, at("plane2.dist")?, *distance);
            put_u8(&mut bytes, at("plane2.type")?, *plane_type);
            put_u8(&mut bytes, at("plane2.signbits")?, *signbits);
            let address = match surface {
                None => 0,
                Some(info) => self.intern_surface(info)?,
            };
            put_u64(&mut bytes, at("surface2")?, address);
        }
        Ok(GuestCallResult::Value(GuestCallValue::Aggregate { layout, bytes }))
    }

    fn intern_surface(&mut self, surface: &TraceSurface) -> Result<u64, HostError> {
        if let Some(address) = self.surfaces.get(surface) {
            return Ok(address.offset);
        }
        let layout = self.surface.clone();
        let address = self
            .memory
            .allocate(&GuestAllocationOptions::bytes(layout.byte_length))?;
        let at = |name: &str| {
            field_offset(&layout, name)
                .map(|offset| offset as i64)
                .map_err(|_| HostError::MissingImport("game".to_string(), name.to_string()))
        };
        let name_bytes = surface.name.as_bytes();
        self.memory.write(address, &name_bytes[..name_bytes.len().min(31)])?;
        self.memory
            .write_u32(self.memory.offset(address, at("flags")?)?, surface.flags)?;
        self.memory
            .write_i32(self.memory.offset(address, at("value")?)?, surface.value)?;
        let id = self.engine.surface_id(surface);
        self.memory.write_u32(self.memory.offset(address, at("id")?)?, id)?;
        let material_bytes = surface.material.as_bytes();
        self.memory.write(
            self.memory.offset(address, at("material")?)?,
            &material_bytes[..material_bytes.len().min(15)],
        )?;
        self.surfaces.insert(surface.clone(), address);
        Ok(address.offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_host() -> RereleaseQ2GuestHost {
        let module = qa_guest::core::contracts::ModuleIdentity::new(
            qa_core::identity::ProviderId::new("q2", "host-test"),
            "game.dll",
            qa_guest::core::contracts::ContentDigest::new("sha256", "00"),
            "test",
        );
        let memory = SparseGuestMemory::new(module, 8, 0x1_0000).expect("memory");
        RereleaseQ2GuestHost::new(
            memory,
            &HostOptions {
                pickups_admitted: false,
                component_projection: false,
                headless_debug: true,
                debug_shapes_bound: false,
                world_text_bound: false,
                foreign_damage: true,
                native_entries: true,
            },
        )
        .expect("host")
    }

    fn trace_fixture() -> Q2Trace {
        Q2Trace {
            all_solid: false,
            start_solid: false,
            fraction: 0.5,
            end: Vec3 { x: 5.0, y: 0.0, z: 0.0 },
            plane_normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            plane_distance: 0.0,
            plane_type: 2,
            plane_signbits: 0,
            surface: Some(TraceSurface {
                name: "metal".to_string(),
                flags: 1,
                value: 0,
                material: "metal".to_string(),
            }),
            contents: 1,
            hit: TraceHit::World,
            secondary: None,
        }
    }

    #[test]
    fn actors_bind_trace_and_link() {
        let mut host = test_host();
        host.init().expect("init");
        host.set_entity_count(4);
        host.set_inuse(1, true).expect("inuse");
        host.set_inuse(2, true).expect("inuse");
        let first = host.actor(1).expect("actor").expect("live");
        let second = host.actor(2).expect("actor").expect("live");
        assert_ne!(first, second);
        host.engine.traces.push_back(trace_fixture());
        let start = host.memory.allocate(&GuestAllocationOptions::bytes(12)).expect("alloc");
        let end = host.memory.allocate(&GuestAllocationOptions::bytes(12)).expect("alloc");
        host.write_vec(
            end,
            Vec3 {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
        )
        .expect("end");
        let record = host.record_at(1).expect("record");
        let encoded = host
            .dispatch(
                "game",
                "trace",
                &[
                    GuestCallValue::Pointer(Some(start)),
                    GuestCallValue::Pointer(None),
                    GuestCallValue::Pointer(None),
                    GuestCallValue::Pointer(Some(end)),
                    GuestCallValue::Pointer(None),
                    GuestCallValue::Uint32(1),
                ],
            )
            .expect("trace");
        let GuestCallResult::Value(GuestCallValue::Aggregate { bytes, .. }) = encoded else {
            panic!("aggregate trace");
        };
        assert_eq!(bytes.len(), 96);
        assert_eq!(bytes[0], 0);
        assert_eq!(f32::from_le_bytes(bytes[4..8].try_into().expect("f")), 0.5);
        assert_eq!(host.engine.trace_queries.len(), 1);
        host.engine.link_meta.insert(first, (3, 0, 31));
        host.dispatch("game", "linkentity", &[GuestCallValue::Pointer(Some(record))])
            .expect("link");
        assert!(host.engine.bodies.get(&first).expect("body").linked);
        host.dispatch("game", "unlinkentity", &[GuestCallValue::Pointer(Some(record))])
            .expect("unlink");
        assert!(!host.engine.bodies.get(&first).expect("body").linked);
        host.shutdown();
    }

    #[test]
    fn box_edicts_core_and_saves() {
        let mut host = test_host();
        host.init().expect("init");
        host.set_entity_count(4);
        host.set_inuse(1, true).expect("inuse");
        let actor = host.actor(1).expect("actor").expect("live");
        host.engine.box_edicts = vec![actor];
        let min = host.memory.allocate(&GuestAllocationOptions::bytes(12)).expect("alloc");
        let max = host.memory.allocate(&GuestAllocationOptions::bytes(12)).expect("alloc");
        let list = host.memory.allocate(&GuestAllocationOptions::bytes(16)).expect("alloc");
        host.set_box_filter(|_| 0);
        let filter = host.memory.allocate(&GuestAllocationOptions::bytes(1)).expect("alloc");
        let count = host
            .dispatch(
                "game",
                "BoxEdicts",
                &[
                    GuestCallValue::Pointer(Some(min)),
                    GuestCallValue::Pointer(Some(max)),
                    GuestCallValue::Pointer(Some(list)),
                    GuestCallValue::Uint64(2),
                    GuestCallValue::Int32(1),
                    GuestCallValue::Pointer(Some(filter)),
                    GuestCallValue::Pointer(None),
                ],
            )
            .expect("box");
        assert_eq!(count, GuestCallResult::Value(GuestCallValue::Uint64(1)));
        let text = host.memory.allocate(&GuestAllocationOptions::bytes(3)).expect("alloc");
        host.memory.write(text, b"hi").expect("write");
        host.dispatch("game", "Com_Print", &[GuestCallValue::Pointer(Some(text))])
            .expect("print");
        assert_eq!(host.engine.printed, vec!["hi".to_string()]);
        host.script_save(b"{}".to_vec());
        let saved = host.write_save("level", false).expect("save");
        assert_eq!(saved.native, b"{}");
        host.read_save(&saved).expect("read");
        let bad = SourceSave {
            native: vec![b'{', 0, b'}'],
            deferred: vec![],
            projections: vec![],
        };
        assert_eq!(host.read_save(&bad).unwrap_err(), HostError::SaveTerminator);
        let reserved = host.reserve_client(3).expect("reserve");
        assert!(host.is_client_reserved(3));
        assert_ne!(reserved, actor);
        host.release_client_reservation(3).expect("release");
        assert!(!host.is_client_reserved(3));
    }
}
