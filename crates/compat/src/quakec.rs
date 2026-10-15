//! One safe version-six execution loop with optional hooks selected per call.
use crate::{
    hooks::Hooks,
    memory::{MemoryError, ModuleMemory},
};
use qa_core::math::native_integer;
use qa_core::{bits::BitSet, primitives::Vec3, random::Random};
use qa_formats::program::quakec::{Image, Opcode};
use std::sync::Arc;

mod client;
mod continuation;
mod map;
pub(crate) mod monster;
mod push;
pub(crate) mod sound;
pub use client::ClientCallRestore;
use continuation::{ActiveRun, RunState, Settings};
pub use continuation::{BuiltinCall, Run, Step};
pub use map::{MapAdmission, MapFields};
pub use qa_session::physics::MotionState;
pub(crate) type EntityStore<'a> = dyn for<'memory> FnMut(
        &mut ModuleMemory<'memory>,
        u64,
        u32,
    ) -> Result<(), crate::services::CallError>
    + 'a;

fn mark_entity_rows(
    dirty: &mut BitSet,
    slots: &mut [usize],
    count: &mut usize,
    first: usize,
    last: usize,
) {
    for slot in first..=last {
        if dirty.insert(slot) {
            slots[*count] = slot;
            *count += 1;
        }
    }
}

fn store_entity_word(
    memory: &mut ModuleMemory<'_>,
    dirty: &mut BitSet,
    slots: &mut [usize],
    count: &mut usize,
    stride: usize,
    address: u64,
    word: u32,
) -> Result<(), Trap> {
    memory.write_word(address, word as i32)?;
    // StoreP accepts an unaligned word crossing a native row boundary.
    mark_entity_rows(
        dirty,
        slots,
        count,
        address as usize / stride,
        (address as usize + 3) / stride,
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trap {
    Statement,
    Function,
    Stack,
    Locals,
    Memory,
    String,
    Builtin,
    Budget,
    World,
    Continuation,
}
impl From<MemoryError> for Trap {
    fn from(_: MemoryError) -> Self {
        Self::Memory
    }
}
pub trait Builtins {
    fn call(&mut self, vm: &mut Vm, number: u32, argc: usize) -> Result<(), Trap>;
    fn stopped(&mut self, vm: &mut Vm, call: &BuiltinCall) -> Result<(), Trap> {
        self.call(vm, call.number, call.argc())
    }
}

/// Native edict ABI data, supplied independently of the map and movement.
#[derive(Clone, Copy)]
pub struct Layout {
    pub entities: usize,
    pub header_bytes: usize,
    pub extra_string_bytes: usize,
    pub state_step: f64,
    pub random_seed: u32,
}
#[derive(Clone, Copy, Default)]
struct Frame {
    function: usize,
    return_pc: usize,
}
struct StateFields {
    nextthink: usize,
    frame: usize,
    think: usize,
}
struct CallFields {
    self_global: Option<usize>,
    other_global: Option<usize>,
    time_global: Option<usize>,
    frame_time: Option<usize>,
}
/// One current native row at its physics phase. This is borrowed ABI
/// classification, not another entity lifetime or movement implementation.
pub struct PhysicsEntity {
    pub entity: qa_core::primitives::EntityId,
    pub native: qa_core::primitives::NativeEntity,
    pub phase: qa_core::primitives::Motion,
}
#[derive(Clone, Copy, Debug)]
pub enum MotionField {
    Flags(u32),
    Ground(qa_core::primitives::NativeEntityRef),
    WaterType(f32),
    WaterLevel(f32),
}
#[derive(Clone, Copy)]
pub(crate) enum Scalar {
    Origin(usize),
    Velocity(usize),
    Angles(usize),
    AngularVelocity(usize),
    Flags,
    Ground,
    IdealYaw,
    Yaw,
    Min(usize),
    Max(usize),
    AbsoluteMin(usize),
    AbsoluteMax(usize),
    LocalTime,
    NextThink,
    MoveType,
    Solid,
}
impl From<qa_session::physics::MotionWrite> for Scalar {
    fn from(value: qa_session::physics::MotionWrite) -> Self {
        use qa_session::physics::MotionWrite;
        match value {
            MotionWrite::Origin(axis) => Self::Origin(axis),
            MotionWrite::Velocity(axis) => Self::Velocity(axis),
            MotionWrite::Angles(axis) => Self::Angles(axis),
            MotionWrite::AngularVelocity(axis) => Self::AngularVelocity(axis),
        }
    }
}

impl From<qa_session::physics::PhysicsError> for crate::services::CallError {
    fn from(error: qa_session::physics::PhysicsError) -> Self {
        match error {
            qa_session::physics::PhysicsError::Entity => Self::Entity,
            qa_session::physics::PhysicsError::Capacity => Self::Capacity,
            qa_session::physics::PhysicsError::AngularVelocity
            | qa_session::physics::PhysicsError::MissingField => Self::Memory,
        }
    }
}
impl From<qa_world::collision::TraceError> for crate::services::CallError {
    fn from(_: qa_world::collision::TraceError) -> Self {
        Self::Geometry
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallPhase {
    Export,
    Think,
    Spawn,
}
struct TraceFields {
    scalars: [usize; 6],
    end: usize,
    normal: usize,
    entity: usize,
}

/// Resolve typed ABI definitions once. Missing or incompatible service fields
/// reject only a builtin that actually needs them, not unrelated bytecode.
pub(crate) fn definition(image: &Image, fields: bool, name: &[u8], kind: u16) -> Option<usize> {
    let name = image.names.find(name)?;
    let definitions = if fields {
        &image.fielddefs
    } else {
        &image.globaldefs
    };
    let definition = definitions
        .iter()
        .find(|definition| definition.name == name)?;
    (definition.value_type & 0x7fff == kind).then_some(definition.offset)
}
pub const TEMP_STRING_BYTES: usize = 128;
/// Original QuakeWorld MAX_INFO_STRING, including its terminating byte.
pub const QW_USERINFO_BYTES: usize = 196;

pub(super) fn temporary_string(memory: &mut ModuleMemory<'_>, text: &[u8]) -> Result<u32, Trap> {
    if text.len() >= TEMP_STRING_BYTES {
        return Err(Trap::String);
    }
    let start = memory
        .len()
        .checked_sub(TEMP_STRING_BYTES)
        .ok_or(Trap::String)?;
    memory.write_string(start as u64, TEMP_STRING_BYTES, text)?;
    u32::try_from(start).map_err(|_| Trap::String)
}

pub struct Vm {
    pub image: Image,
    pub entities: ModuleMemory<'static>,
    pub strings: ModuleMemory<'static>,
    pub hooks: Hooks,
    pub active: bool,
    pub argc: usize,
    pub statement: usize,
    pub stride: usize,
    pub header_bytes: usize,
    pub(crate) projection: crate::entities::EntityProjection,
    dirty_entities: BitSet,
    dirty_slots: Box<[usize]>,
    dirty_count: usize,
    state: Option<StateFields>,
    call_fields: CallFields,
    map_globals: map::MapGlobals,
    client_globals: client::ClientGlobals,
    trace_fields: Option<TraceFields>,
    vector_globals: Option<[usize; 3]>,
    state_step: f64,
    random: Random,
    stack: [Frame; 32],
    locals: [u32; 2048],
    depth: usize,
    locals_used: usize,
    string_used: usize,
    string_limit: usize,
    temporary_used: usize,
    run_owner: Arc<()>,
    next_run: u64,
    active_run: Option<ActiveRun>,
    settings: Settings,
}
impl Vm {
    pub fn load(image: Image, layout: Layout) -> Result<Self, Trap> {
        if layout.entities == 0
            || !layout.header_bytes.is_multiple_of(4)
            || !layout.state_step.is_finite()
        {
            return Err(Trap::Memory);
        }
        let stride = layout
            .header_bytes
            .checked_add(image.entityfields.checked_mul(4).ok_or(Trap::Memory)?)
            .ok_or(Trap::Memory)?;
        if stride == 0 {
            return Err(Trap::Memory);
        }
        let extent = stride
            .checked_mul(layout.entities)
            .filter(|&n| n <= i32::MAX as usize)
            .ok_or(Trap::Memory)?;
        let entities = ModuleMemory::load(0, extent, &[])?;
        let string_used = image.strings.len();
        let string_limit = string_used
            .checked_add(layout.extra_string_bytes)
            .ok_or(Trap::Memory)?;
        let strings = ModuleMemory::load(
            0,
            string_limit
                .checked_add(TEMP_STRING_BYTES)
                .filter(|&n| n <= i32::MAX as usize)
                .ok_or(Trap::Memory)?,
            &image.strings,
        )?;
        let state = (|| {
            Some(StateFields {
                nextthink: definition(&image, true, b"nextthink", 2)?,
                frame: definition(&image, true, b"frame", 2)?,
                think: definition(&image, true, b"think", 6)?,
            })
        })();
        let call_fields = CallFields {
            self_global: definition(&image, false, b"self", 4),
            other_global: definition(&image, false, b"other", 4),
            time_global: definition(&image, false, b"time", 2),
            frame_time: definition(&image, false, b"frametime", 2),
        };
        let map_globals = map::MapGlobals::load(&image);
        let client_globals = client::ClientGlobals::load(&image);
        let hook_bytes = image
            .globals
            .len()
            .checked_mul(4)
            .and_then(|b| b.checked_add(extent))
            .ok_or(Trap::Memory)?;
        let vector_globals = (|| {
            let offsets = [
                image.global(b"v_forward")?,
                image.global(b"v_right")?,
                image.global(b"v_up")?,
            ];
            offsets
                .iter()
                .all(|&offset| {
                    offset
                        .checked_add(3)
                        .is_some_and(|end| end <= image.globals.len())
                })
                .then_some(offsets)
        })();
        let trace_fields = (|| {
            Some(TraceFields {
                scalars: [
                    definition(&image, false, b"trace_allsolid", 2)?,
                    definition(&image, false, b"trace_startsolid", 2)?,
                    definition(&image, false, b"trace_fraction", 2)?,
                    definition(&image, false, b"trace_inwater", 2)?,
                    definition(&image, false, b"trace_inopen", 2)?,
                    definition(&image, false, b"trace_plane_dist", 2)?,
                ],
                end: definition(&image, false, b"trace_endpos", 3)?,
                normal: definition(&image, false, b"trace_plane_normal", 3)?,
                entity: definition(&image, false, b"trace_ent", 4)?,
            })
        })();
        let projection = crate::entities::EntityProjection::load_quakec(
            &image,
            stride,
            layout.header_bytes,
            layout.entities,
        );
        Ok(Self {
            image,
            entities,
            strings,
            hooks: Hooks::load(hook_bytes),
            active: false,
            argc: 0,
            statement: 0,
            stride,
            header_bytes: layout.header_bytes,
            projection,
            dirty_entities: BitSet::empty(layout.entities),
            dirty_slots: vec![0; layout.entities].into_boxed_slice(),
            dirty_count: 0,
            state,
            call_fields,
            map_globals,
            client_globals,
            trace_fields,
            vector_globals,
            state_step: layout.state_step,
            random: Random::with_algorithm(
                layout.random_seed,
                qa_core::random::Algorithm::GlibcType3,
            ),
            stack: [Frame::default(); 32],
            locals: [0; 2048],
            depth: 0,
            locals_used: 0,
            string_used,
            string_limit,
            temporary_used: 0,
            run_owner: Arc::new(()),
            next_run: 0,
            active_run: None,
            settings: Settings::default(),
        })
    }
    pub fn random_word(&mut self) -> u32 {
        self.random.next_u32()
    }
    pub fn seed_entities(
        &mut self,
        entities: &qa_world::entities::EntityTable,
        module: qa_core::primitives::ModuleId,
        rules: qa_core::primitives::RuleSetId,
        max_clients: i32,
    ) -> Result<(), crate::services::CallError> {
        self.projection
            .quakec_seed(entities, module, rules, max_clients)?;
        self.client_globals.rules = Some(rules);
        Ok(())
    }
    /// Cold reservation for retained common names across this module's map
    /// lifetime. Reused temporary strings and suffixes can exhaust this fixed
    /// history budget; publication then returns Capacity without growing it.
    pub fn name_history_reservation(&self) -> Result<(usize, usize), Trap> {
        let fields = (self.entities.len() / self.stride)
            .checked_mul(3)
            .ok_or(Trap::Memory)?;
        let names = fields
            .checked_add(self.string_limit.div_ceil(TEMP_STRING_BYTES))
            .ok_or(Trap::Memory)?;
        let bytes = self
            .string_limit
            .checked_mul(4)
            .and_then(|bytes| fields.checked_mul(TEMP_STRING_BYTES)?.checked_add(bytes))
            .ok_or(Trap::Memory)?;
        Ok((names, bytes))
    }
    /// Cache the native function-to-session callback conversion once at load.
    pub fn bind_thinks(&mut self, binding: qa_session::dispatch::ModuleBinding, entries: &[u32]) {
        self.projection
            .quakec_bind_thinks(binding, entries, self.image.functions.len());
    }
    pub fn needs_self(&self) -> bool {
        self.call_fields.self_global.is_some()
    }
    pub fn needs_world(&self) -> bool {
        self.call_fields.other_global.is_some()
    }
    /// Set the native per-call globals and clear a due native deadline before
    /// running bytecode. World is the explicitly bound native world entity.
    pub fn prepare_call(
        &mut self,
        entity: Option<u32>,
        world: Option<u32>,
        time: qa_core::primitives::ThinkTime,
        phase: CallPhase,
    ) -> Result<(), Trap> {
        if phase == CallPhase::Think {
            self.prepare_think(entity.ok_or(Trap::Memory)?, time)
                .map_err(|_| Trap::Memory)?;
            return Ok(());
        }
        if let Some(global) = self.call_fields.self_global {
            self.image.globals[global] = entity.ok_or(Trap::Memory)?;
        }
        if phase == CallPhase::Spawn {
            return Ok(());
        }
        if let Some(global) = self.call_fields.other_global {
            self.image.globals[global] = world.ok_or(Trap::Memory)?;
        }
        if self.call_fields.time_global.is_some() {
            self.set_time(time).map_err(|_| Trap::Memory)?;
        }
        Ok(())
    }
    pub fn frame_time(&mut self, step: f64) {
        if let Some(global) = self.call_fields.frame_time {
            self.image.globals[global] = (step as f32).to_bits();
        }
    }
    /// Advance through the existing native namespace, rechecking canonical
    /// lifetime liveness at each row. Later spawns join the same native pass;
    /// an earlier reused ordinal is visited on the next pass.
    pub fn physics_entity(
        &self,
        entities: &qa_world::entities::EntityTable,
        cursor: &mut usize,
    ) -> Result<Option<PhysicsEntity>, crate::services::CallError> {
        self.projection.quakec_physics_entity(entities, cursor)
    }
    /// Resolve the currently live module-owned lifetime of a captured native
    /// row. Native reuse changes its common handle, never revives the old one.
    pub fn bound_entity(
        &self,
        entities: &qa_world::entities::EntityTable,
        native: qa_core::primitives::NativeEntity,
    ) -> Result<Option<qa_core::primitives::EntityId>, crate::services::CallError> {
        self.projection.quakec_bound_entity(entities, native)
    }
    /// Materialize an unconnected native client's ordinary common body.
    pub fn ensure_client_body(
        &mut self,
        services: &mut crate::services::EngineServices<'_>,
        context: crate::services::CallContext,
        native: u32,
    ) -> Result<qa_core::primitives::EntityId, crate::services::CallError> {
        self.projection.quakec_ensure_client_body(
            services,
            &self.entities,
            &self.strings,
            context,
            native,
        )
    }
    /// Preserve native backing when its borrowed connection lifetime ends.
    pub fn retain_client_body(
        &mut self,
        services: &mut crate::services::EngineServices<'_>,
        context: crate::services::CallContext,
        native: u32,
        old: qa_core::primitives::EntityId,
    ) -> Result<qa_core::primitives::EntityId, crate::services::CallError> {
        self.projection.quakec_retain_client_body(
            services,
            &self.entities,
            &self.strings,
            context,
            native,
            old,
        )
    }
    /// Retire only this reserved native client's owned orphan body.
    pub fn release_client_body(
        &mut self,
        services: &mut crate::services::EngineServices<'_>,
        context: crate::services::CallContext,
        native: u32,
    ) -> Result<(), crate::services::CallError> {
        self.projection
            .quakec_release_client_body(services, &self.entities, context, native)
    }
    /// Map admission and builtin spawn use the existing native ordinal
    /// allocator. This does not change the VM's return globals.
    pub fn spawn_entity(
        &mut self,
        services: &mut crate::services::EngineServices<'_>,
        context: crate::services::CallContext,
    ) -> Result<u32, crate::services::CallError> {
        self.projection
            .quakec_spawn(services, &mut self.entities, context)
    }
    pub fn remove_entity(
        &mut self,
        services: &mut crate::services::EngineServices<'_>,
        context: crate::services::CallContext,
        native: u32,
    ) -> Result<(), crate::services::CallError> {
        self.projection
            .quakec_remove(services, &mut self.entities, context, native)
    }
    pub fn make_static(
        &mut self,
        services: &mut crate::services::EngineServices<'_>,
        context: crate::services::CallContext,
        native: u32,
    ) -> Result<(), crate::services::CallError> {
        self.projection.quakec_make_static(
            services,
            &mut self.entities,
            &self.strings,
            context,
            native,
        )
    }
    pub fn resolve_entity(
        &self,
        entities: &qa_world::entities::EntityTable,
        module: qa_core::primitives::ModuleId,
        native: u32,
    ) -> Result<Option<qa_core::primitives::EntityId>, crate::services::CallError> {
        let slot = self
            .entity_number(native)
            .map_err(|_| crate::services::CallError::Memory)?;
        self.bound_entity(
            entities,
            qa_core::primitives::NativeEntity {
                module,
                slot: slot as i32,
            },
        )
    }
    pub(crate) fn find_entity(
        &self,
        entities: &qa_world::entities::EntityTable,
        module: qa_core::primitives::ModuleId,
        start: u32,
        field: u32,
        match_offset: i32,
    ) -> Result<u32, crate::services::CallError> {
        use crate::services::CallError;
        let ordinal = self.entity_number(start).map_err(|_| CallError::Memory)?;
        let mut cursor = self.projection.quakec_find_start(module, ordinal)?;
        let text = self.string(match_offset).map_err(|_| CallError::Memory)?;
        while let Some((native, _)) = self.projection.quakec_next_entity(entities, &mut cursor)? {
            let native = (native.slot as usize)
                .checked_mul(self.stride)
                .and_then(|offset| u32::try_from(offset).ok())
                .ok_or(CallError::Memory)?;
            // PF_Find takes a raw field word offset, without a definition or
            // classname lookup. Check only the native field it actually reads.
            let address = self
                .field_address(native, field, 1)
                .map_err(|_| CallError::Memory)?;
            let offset = self.entities.read_word(address)?;
            if self.string(offset).map_err(|_| CallError::Memory)? == text {
                return Ok(native);
            }
        }
        Ok(0)
    }
    /// Change only the native self yaw, using the same dirty-row tracking as a
    /// bytecode scalar store. Unlinked and freed native backing stays addressable.
    pub fn change_yaw(&mut self) -> Result<(), crate::services::CallError> {
        let global = self
            .call_fields
            .self_global
            .ok_or(crate::services::CallError::Memory)?;
        let native = self.image.globals[global];
        if let Some((address, value)) = self.projection.quakec_yaw_change(&self.entities, native)? {
            self.store_entity_word(address, value.to_bits())
                .map_err(|_| crate::services::CallError::Memory)?;
        }
        Ok(())
    }
    pub fn check_bottom(
        &self,
        services: &mut crate::services::EngineServices<'_>,
        context: crate::services::CallContext,
        native: u32,
    ) -> Result<bool, crate::services::CallError> {
        self.projection.quakec_check_bottom(
            services,
            &self.entities,
            &self.strings,
            context,
            native,
        )
    }
    /// Native droptofloor reads the current self row and uses the shared
    /// caller-selected trace and explicit no-touch entity link service.
    pub fn drop_to_floor(
        &mut self,
        services: &mut crate::services::EngineServices<'_>,
        context: crate::services::CallContext,
    ) -> Result<bool, crate::services::CallError> {
        let global = self
            .call_fields
            .self_global
            .ok_or(crate::services::CallError::Memory)?;
        let entity = self.image.globals[global];
        let dropped = self.with_entity_stores(|projection, memory, strings, store| {
            projection.quakec_drop_to_floor(services, memory, strings, context, entity, store)
        })?;
        if dropped {
            let slot = entity as usize / self.stride;
            self.mark_entity_rows(slot, slot);
        }
        Ok(dropped)
    }
    /// Refresh the angular input for the shared entity executor. Missing
    /// native fields reject only this executed motion path.
    pub fn prepare_motion(
        &mut self,
        entities: &mut qa_world::entities::EntityTable,
        context: crate::services::CallContext,
        value: u32,
    ) -> Result<(), crate::services::CallError> {
        self.projection
            .quakec_prepare_motion(entities, &self.entities, context, value)
    }
    pub fn motion_state(
        &self,
        context: crate::services::CallContext,
        value: u32,
    ) -> Result<qa_session::physics::MotionState, crate::services::CallError> {
        self.projection
            .quakec_motion_state(&self.entities, context, value)
    }
    pub fn motion_gravity(
        &self,
        context: crate::services::CallContext,
        value: u32,
    ) -> Result<Option<f32>, crate::services::CallError> {
        self.projection
            .quakec_motion_gravity(&self.entities, context, value)
    }
    pub fn motion_water_type(
        &self,
        context: crate::services::CallContext,
        value: u32,
    ) -> Result<f32, crate::services::CallError> {
        self.projection
            .quakec_motion_water_type(&self.entities, context, value)
    }
    pub fn with_trace_view<R>(
        &self,
        context: crate::services::CallContext,
        pass: u32,
        query: impl FnOnce(
            &dyn qa_world::collision::NativeTraceEntities,
            qa_core::primitives::CollisionOwner,
        ) -> Result<R, crate::services::CallError>,
    ) -> Result<R, crate::services::CallError> {
        let (view, pass) =
            self.projection
                .quakec_trace_view(&self.entities, &self.strings, context, pass)?;
        let result = query(&view, pass);
        view.finish()?;
        result
    }
    pub fn write_motion_field(
        &mut self,
        entities: &mut qa_world::entities::EntityTable,
        context: crate::services::CallContext,
        value: u32,
        field: MotionField,
    ) -> Result<(), crate::services::CallError> {
        let ground = if let MotionField::Ground(reference) = field {
            Some(self.weak_entity_offset(context.module, reference)?)
        } else {
            None
        };
        self.projection.quakec_write_motion_field(
            entities,
            &mut self.entities,
            context,
            value,
            field,
            ground,
        )
    }
    /// Scalar native writes retain VectorMA's read/write overlap order; all
    /// motion arithmetic stays in the shared executor over common columns.
    pub fn write_motion(
        &mut self,
        entities: &mut qa_world::entities::EntityTable,
        context: crate::services::CallContext,
        value: u32,
        stage: qa_session::physics::MotionWrite,
    ) -> Result<(), crate::services::CallError> {
        self.with_entity_stores(|projection, memory, _, store| {
            projection.quakec_write_motion(entities, memory, context, value, stage, store)
        })
    }
    /// Finish the existing native explicit relink without trigger touches;
    /// native world0 only publishes its pose.
    pub fn relink_entity(
        &mut self,
        services: &mut crate::services::EngineServices<'_>,
        context: crate::services::CallContext,
        value: u32,
    ) -> Result<(), crate::services::CallError> {
        self.with_entity_stores(|projection, memory, strings, store| {
            projection.quakec_relink_entity(services, memory, strings, context, value, store)
        })
    }
    /// Publish the already committed area bounds without relinking or moving.
    pub fn publish_link_bounds(
        &mut self,
        services: &mut crate::services::EngineServices<'_>,
        context: crate::services::CallContext,
        value: u32,
    ) -> Result<(), crate::services::CallError> {
        self.with_entity_stores(|projection, memory, strings, store| {
            projection.quakec_publish_link_bounds(services, memory, strings, context, value, store)
        })
    }
    pub(crate) fn with_entity_stores<R>(
        &mut self,
        call: impl FnOnce(
            &mut crate::entities::EntityProjection,
            &mut ModuleMemory<'static>,
            &ModuleMemory<'static>,
            &mut EntityStore<'_>,
        ) -> Result<R, crate::services::CallError>,
    ) -> Result<R, crate::services::CallError> {
        let stride = self.stride;
        let dirty = &mut self.dirty_entities;
        let slots = &mut self.dirty_slots;
        let count = &mut self.dirty_count;
        let mut store = |memory: &mut ModuleMemory<'_>, address, word| {
            store_entity_word(memory, dirty, slots, count, stride, address, word)
                .map_err(|_| crate::services::CallError::Memory)
        };
        call(
            &mut self.projection,
            &mut self.entities,
            &self.strings,
            &mut store,
        )
    }
    /// Import bytecode field stores without replacing explicit native relinks.
    /// Scheduling remains owned by the provider's native think phase.
    pub fn project_entities(
        &mut self,
        services: &mut crate::services::EngineServices<'_>,
        context: crate::services::CallContext,
    ) -> Result<(), crate::services::CallError> {
        self.sync_client_globals()?;
        self.projection
            .quakec_project(services, &self.entities, &self.strings, context)?;
        self.dirty_entities.clear();
        self.dirty_count = 0;
        Ok(())
    }
    /// Publish one native row without consuming other pending guest stores.
    pub fn project_entity(
        &mut self,
        services: &mut crate::services::EngineServices<'_>,
        context: crate::services::CallContext,
        value: u32,
    ) -> Result<(), crate::services::CallError> {
        self.sync_client_globals()?;
        let slot = self
            .entity_number(value)
            .map_err(|_| crate::services::CallError::Memory)?;
        self.projection.quakec_project_row(
            services,
            &self.entities,
            &self.strings,
            context,
            slot as usize,
        )
    }
    /// Module return imports successful bytecode entity writes since the
    /// previous publication. Export projection also catches external byte edits.
    pub fn project_dirty_entities(
        &mut self,
        services: &mut crate::services::EngineServices<'_>,
        context: crate::services::CallContext,
    ) -> Result<(), crate::services::CallError> {
        self.sync_client_globals()?;
        for index in 0..self.dirty_count {
            let slot = self.dirty_slots[index];
            self.projection.quakec_project_row(
                services,
                &self.entities,
                &self.strings,
                context,
                slot,
            )?;
        }
        // Keep the entire pending set on a rejected import so later stores
        // cannot append a cleared row a second time beyond the fixed capacity.
        for &slot in &self.dirty_slots[..self.dirty_count] {
            self.dirty_entities.remove(slot);
        }
        self.dirty_count = 0;
        Ok(())
    }
    fn store_entity_word(&mut self, address: u64, word: u32) -> Result<(), Trap> {
        store_entity_word(
            &mut self.entities,
            &mut self.dirty_entities,
            &mut self.dirty_slots,
            &mut self.dirty_count,
            self.stride,
            address,
            word,
        )
    }
    fn mark_entity_rows(&mut self, first: usize, last: usize) {
        mark_entity_rows(
            &mut self.dirty_entities,
            &mut self.dirty_slots,
            &mut self.dirty_count,
            first,
            last,
        );
    }
    pub(crate) fn write_trace(
        &mut self,
        trace: qa_world::collision::Trace,
        entity: u32,
    ) -> Result<(), Trap> {
        let fields = self.trace_fields.as_ref().ok_or(Trap::Memory)?;
        let values = [
            f32::from(u8::from(trace.all_solid)),
            f32::from(u8::from(trace.start_solid)),
            trace.fraction,
            f32::from(u8::from(trace.in_water)),
            f32::from(u8::from(trace.in_open)),
            trace.plane.distance,
        ];
        for (&offset, value) in fields.scalars.iter().zip(values) {
            self.image.globals[offset] = value.to_bits();
        }
        let end = fields.end;
        let normal = fields.normal;
        self.image.globals[fields.entity] = entity;
        self.write_global_vector(end, trace.end)?;
        self.write_global_vector(normal, trace.plane.normal)
    }
    pub(crate) fn write_global_vector(&mut self, offset: usize, value: Vec3) -> Result<(), Trap> {
        let end = offset.checked_add(3).ok_or(Trap::Memory)?;
        self.image
            .globals
            .get_mut(offset..end)
            .ok_or(Trap::Memory)?
            .copy_from_slice(&value.0.map(f32::to_bits));
        Ok(())
    }
    pub(crate) fn write_basis(&mut self, basis: qa_core::math::AngleBasis) -> Result<(), Trap> {
        let offsets = self.vector_globals.ok_or(Trap::Memory)?;
        for (offset, value) in offsets
            .into_iter()
            .zip([basis.forward, basis.right, basis.up])
        {
            self.write_global_vector(offset, value)?;
        }
        Ok(())
    }
    pub fn string(&self, offset: i32) -> Result<&[u8], Trap> {
        let address = u64::try_from(offset).map_err(|_| Trap::String)?;
        let end = if address < self.string_used as u64 {
            self.string_used
        } else if address >= self.string_limit as u64
            && address < (self.string_limit + self.temporary_used) as u64
        {
            self.string_limit + self.temporary_used
        } else {
            return Err(Trap::String);
        };
        let tail = self.strings.read(address, end - address as usize)?;
        qa_core::text::terminated(tail).ok_or(Trap::String)
    }
    pub(super) fn register_temporary_string(&mut self, length: u8) {
        self.temporary_used = usize::from(length) + 1;
    }
    pub fn insert_string(&mut self, text: &[u8]) -> Result<i32, Trap> {
        let end = self
            .string_used
            .checked_add(text.len())
            .and_then(|n| n.checked_add(1))
            .filter(|&n| n <= i32::MAX as usize && n <= self.string_limit)
            .ok_or(Trap::String)?;
        let address = self.string_used;
        self.strings.write(address as u64, text)?;
        self.strings.write((end - 1) as u64, &[0])?;
        self.string_used = end;
        Ok(address as i32)
    }
    /// Native entity values are byte offsets into this backing, never common
    /// EntityIds or client ordinals. Engine views read these same bytes.
    pub fn entity_number(&self, entity: u32) -> Result<u32, Trap> {
        self.field_address(entity, 0, 0)?;
        Ok((entity as usize / self.stride) as u32)
    }
    /// A touch's other identity remains weak after the first callback frees
    /// its row. Validate its native namespace and backing, never its liveness.
    pub fn weak_entity_offset(
        &self,
        module: qa_core::primitives::ModuleId,
        reference: qa_core::primitives::NativeEntityRef,
    ) -> Result<u32, crate::services::CallError> {
        use qa_core::primitives::ResourceNamespace;
        if reference.namespace != ResourceNamespace::Module(module) {
            return Err(crate::services::CallError::Entity);
        }
        let value = usize::try_from(reference.ordinal)
            .ok()
            .and_then(|ordinal| ordinal.checked_mul(self.stride))
            .and_then(|value| u32::try_from(value).ok())
            .ok_or(crate::services::CallError::Entity)?;
        self.entity_number(value)
            .map_err(|_| crate::services::CallError::Entity)?;
        Ok(value)
    }
    pub fn field_address(&self, entity: u32, field: u32, width: usize) -> Result<u64, Trap> {
        let row = entity as usize;
        if !row.is_multiple_of(self.stride)
            || row >= self.entities.len()
            || (field as usize)
                .checked_add(width)
                .is_none_or(|end| end > self.image.entityfields)
        {
            return Err(Trap::Memory);
        }
        Ok((row + self.header_bytes + field as usize * 4) as u64)
    }
    pub fn call(
        &mut self,
        builtins: &mut impl Builtins,
        function: u32,
        budget: u64,
        hooks: bool,
    ) -> Result<[u32; 3], Trap> {
        let mut run = self.begin_root(function, budget, hooks)?;
        let mut step = self.advance(&mut run)?;
        loop {
            match step {
                Step::Returned(value) => return Ok(value),
                Step::Builtin(call) => {
                    if let Err(error) = builtins.stopped(self, &call) {
                        self.abort(&mut run)?;
                        return Err(error);
                    }
                    step = self.resume(&mut run)?;
                }
            }
        }
    }
    fn push(&mut self, function: usize, return_pc: usize) -> Result<usize, Trap> {
        let f = self
            .image
            .functions
            .get(function)
            .filter(|_| function != 0)
            .ok_or(Trap::Function)?;
        if f.first_statement < 0 || f.first_statement as usize >= self.image.statements.len() {
            return Err(Trap::Function);
        }
        if self.depth + 1 >= self.stack.len() {
            return Err(Trap::Stack);
        }
        let end = self
            .locals_used
            .checked_add(f.locals)
            .filter(|&n| n <= self.locals.len())
            .ok_or(Trap::Locals)?;
        self.locals[self.locals_used..end]
            .copy_from_slice(&self.image.globals[f.parm_start..f.parm_start + f.locals]);
        self.locals_used = end;
        self.stack[self.depth] = Frame {
            function,
            return_pc,
        };
        self.depth += 1;
        let mut target = f.parm_start;
        for i in 0..f.numparms {
            for j in 0..usize::from(f.parm_size[i]) {
                self.image.globals[target] = self.image.globals[4 + i * 3 + j];
                target += 1;
            }
        }
        Ok(f.first_statement as usize)
    }
    fn leave(&mut self) -> usize {
        self.depth -= 1;
        let frame = self.stack[self.depth];
        let f = &self.image.functions[frame.function];
        self.locals_used -= f.locals;
        self.image.globals[f.parm_start..f.parm_start + f.locals]
            .copy_from_slice(&self.locals[self.locals_used..self.locals_used + f.locals]);
        frame.return_pc
    }
    fn run<const HOOKS: bool>(&mut self, state: &mut RunState) -> Result<Step, Trap> {
        use Opcode::*;
        let mut pc = state.pc;
        loop {
            if self.settings.remaining <= 1 {
                return Err(Trap::Budget);
            }
            self.settings.remaining -= 1;
            let index = pc;
            let statement = *self.image.statements.get(index).ok_or(Trap::Statement)?;
            pc += 1;
            let op = statement.opcode;
            if op == Opcode::Invalid {
                self.statement = index;
                return Err(Trap::Statement);
            }
            let [a, b, c] = statement.operands.map(|v| v as usize);
            if HOOKS {
                self.statement = index;
                self.hooks.instructions += 1;
            }
            match op {
                Done | Return => {
                    for i in 0..3 {
                        self.image.globals[1 + i] = self.image.globals[a + i];
                    }
                    pc = self.leave();
                    if self.depth == state.exit_depth {
                        return Ok(Step::Returned([
                            self.image.globals[1],
                            self.image.globals[2],
                            self.image.globals[3],
                        ]));
                    }
                }
                If | IfNot => {
                    if (self.image.globals[a] != 0) == (op == If) {
                        pc = (index as i64 + i64::from(statement.operands[1])) as usize;
                    }
                }
                Goto => pc = (index as i64 + i64::from(statement.operands[0])) as usize,
                Call0 | Call1 | Call2 | Call3 | Call4 | Call5 | Call6 | Call7 | Call8 => {
                    self.argc = op as usize - Call0 as usize;
                    let target = self.image.globals[a] as usize;
                    let f = self
                        .image
                        .functions
                        .get(target)
                        .filter(|_| target != 0)
                        .ok_or(Trap::Function)?;
                    if HOOKS {
                        self.hooks.calls += 1;
                    }
                    if f.first_statement < 0 {
                        self.statement = index;
                        let number = f.first_statement.wrapping_neg() as u32;
                        let call = BuiltinCall::capture(self, number, self.argc)?;
                        state.pc = pc;
                        return Ok(Step::Builtin(call));
                    } else {
                        pc = self.push(target, pc)?;
                    }
                }
                StoreF | StoreS | StoreEnt | StoreFld | StoreFnc | StoreV => {
                    let width = if op == StoreV { 3 } else { 1 };
                    // Copy in native component order, including overlapping globals.
                    for i in 0..width {
                        self.image.globals[b + i] = self.image.globals[a + i];
                    }
                    if HOOKS {
                        self.hooks.stores += 1;
                        self.hooks.write(b * 4, width * 4);
                    }
                }
                StorePF | StorePS | StorePEnt | StorePFld | StorePFnc | StorePV => {
                    let width = if op == StorePV { 3 } else { 1 };
                    let address = u64::from(self.image.globals[b]);
                    self.entities.read(address, width * 4)?;
                    for i in 0..width {
                        self.store_entity_word(address + i as u64 * 4, self.image.globals[a + i])?;
                    }
                    if HOOKS {
                        self.hooks.stores += 1;
                        self.hooks
                            .write(self.image.globals.len() * 4 + address as usize, width * 4);
                    }
                }
                Address | LoadF | LoadS | LoadEnt | LoadFld | LoadFnc | LoadV => {
                    let entity = self.image.globals[a];
                    let field = self.image.globals[b];
                    let width = if op == LoadV { 3 } else { 1 };
                    let address = self.field_address(entity, field, width)?;
                    if op == Address {
                        if entity == 0 && self.active {
                            return Err(Trap::World);
                        }
                        self.image.globals[c] = address as u32;
                    } else {
                        for i in 0..width {
                            self.image.globals[c + i] =
                                self.entities.read_word(address + i as u64 * 4)? as u32;
                        }
                    }
                    if HOOKS {
                        self.hooks.stores += 1;
                        self.hooks.write(c * 4, width * 4);
                    }
                }
                State => {
                    let state = self.state.as_ref().ok_or(Trap::Memory)?;
                    let entity =
                        self.image.globals[self.call_fields.self_global.ok_or(Trap::Memory)?];
                    let nextthink = self.field_address(entity, state.nextthink as u32, 1)?;
                    let frame = self.field_address(entity, state.frame as u32, 1)?;
                    let think = self.field_address(entity, state.think as u32, 1)?;
                    let time = f32::from_bits(
                        self.image.globals[self.call_fields.time_global.ok_or(Trap::Memory)?],
                    );
                    self.store_entity_word(
                        nextthink,
                        ((time as f64 + self.state_step) as f32).to_bits(),
                    )?;
                    if f32::from_bits(self.image.globals[a])
                        != f32::from_bits(self.entities.read_word(frame)? as u32)
                    {
                        self.store_entity_word(frame, self.image.globals[a])?;
                    }
                    self.store_entity_word(think, self.image.globals[b])?;
                    if HOOKS {
                        self.hooks.stores += 1;
                        for address in [nextthink, frame, think] {
                            self.hooks
                                .write(self.image.globals.len() * 4 + address as usize, 4);
                        }
                    }
                }
                AddV | SubV | MulFV | MulVF => {
                    for i in 0..3 {
                        let av =
                            f32::from_bits(self.image.globals[a + if op == MulFV { 0 } else { i }]);
                        let bv =
                            f32::from_bits(self.image.globals[b + if op == MulVF { 0 } else { i }]);
                        self.image.globals[c + i] = (match op {
                            AddV => av + bv,
                            SubV => av - bv,
                            _ => av * bv,
                        })
                        .to_bits();
                    }
                    if HOOKS {
                        self.hooks.stores += 1;
                        self.hooks.write(c * 4, 12);
                    }
                }
                MulV | EqV | NeV | NotV => {
                    let av = [0, 1, 2].map(|i| f32::from_bits(self.image.globals[a + i]));
                    let bv = if op == NotV {
                        [0.0; 3]
                    } else {
                        [0, 1, 2].map(|i| f32::from_bits(self.image.globals[b + i]))
                    };
                    self.image.globals[c] = (match op {
                        MulV => Vec3(av).dot(Vec3(bv)),
                        EqV => boolean(av == bv),
                        NeV => boolean(av != bv),
                        _ => boolean(av == [0.0; 3]),
                    })
                    .to_bits();
                    if HOOKS {
                        self.hooks.stores += 1;
                        self.hooks.write(c * 4, 4);
                    }
                }
                _ => {
                    let aw = self.image.globals[a];
                    let bw = if matches!(op, NotF | NotS | NotEnt | NotFnc) {
                        0
                    } else {
                        self.image.globals[b]
                    };
                    let af = f32::from_bits(aw);
                    let bf = f32::from_bits(bw);
                    let value = match op {
                        MulF => af * bf,
                        DivF => af / bf,
                        AddF => af + bf,
                        SubF => af - bf,
                        EqF => boolean(af == bf),
                        NeF => boolean(af != bf),
                        Le => boolean(af <= bf),
                        Ge => boolean(af >= bf),
                        Lt => boolean(af < bf),
                        Gt => boolean(af > bf),
                        EqE | EqFnc => boolean(aw == bw),
                        NeE | NeFnc => boolean(aw != bw),
                        And => boolean(af != 0.0 && bf != 0.0),
                        Or => boolean(af != 0.0 || bf != 0.0),
                        NotF => boolean(af == 0.0),
                        NotEnt | NotFnc => boolean(aw == 0),
                        NotS => boolean(aw == 0 || self.string(aw as i32)?.is_empty()),
                        EqS => boolean(self.string(aw as i32)? == self.string(bw as i32)?),
                        NeS => {
                            string_compare(self.string(aw as i32)?, self.string(bw as i32)?) as f32
                        }
                        BitAnd => (native_integer(af) & native_integer(bf)) as f32,
                        BitOr => (native_integer(af) | native_integer(bf)) as f32,
                        _ => return Err(Trap::Statement),
                    };
                    self.image.globals[c] = value.to_bits();
                    if HOOKS {
                        self.hooks.stores += 1;
                        self.hooks.write(c * 4, 4);
                    }
                }
            }
        }
    }
}
fn boolean(value: bool) -> f32 {
    if value { 1.0 } else { 0.0 }
}
fn string_compare(a: &[u8], b: &[u8]) -> i32 {
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return i32::from(x) - i32::from(y);
        }
    }
    0
}
