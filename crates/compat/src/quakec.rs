//! One safe version-six execution loop with optional hooks selected per call.
use crate::numbers::native_integer;
use crate::{
    hooks::Hooks,
    memory::{MemoryError, ModuleMemory},
};
use qa_core::{bits::BitSet, primitives::Vec3};
use qa_formats::program::quakec::{Image, Opcode};

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
}
impl From<MemoryError> for Trap {
    fn from(_: MemoryError) -> Self {
        Self::Memory
    }
}
pub trait Builtins {
    fn call(&mut self, vm: &mut Vm, number: u32, argc: usize) -> Result<(), Trap>;
}

/// Native edict ABI data, supplied independently of the map and movement.
#[derive(Clone, Copy)]
pub struct Layout {
    pub entities: usize,
    pub header_bytes: usize,
    pub extra_string_bytes: usize,
    pub state_step: f64,
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
    pub none: bool,
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
    trace_fields: Option<TraceFields>,
    vector_globals: Option<[usize; 3]>,
    state_step: f64,
    stack: [Frame; 32],
    locals: [u32; 2048],
    depth: usize,
    locals_used: usize,
    string_used: usize,
    string_limit: usize,
    temporary_used: usize,
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
            trace_fields,
            vector_globals,
            state_step: layout.state_step,
            stack: [Frame::default(); 32],
            locals: [0; 2048],
            depth: 0,
            locals_used: 0,
            string_used,
            string_limit,
            temporary_used: 0,
        })
    }
    pub fn seed_entities(
        &mut self,
        entities: &qa_world::entities::EntityTable,
        module: qa_core::primitives::ModuleId,
    ) -> Result<(), crate::services::CallError> {
        self.projection.seed(entities, module)
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
        think: bool,
    ) -> Result<(), Trap> {
        if think {
            self.projection
                .quakec_clear_think(&mut self.entities, entity.ok_or(Trap::Memory)?)
                .map_err(|_| Trap::Memory)?;
        }
        for (global, value) in [
            (self.call_fields.self_global, entity),
            (self.call_fields.other_global, world),
        ] {
            if let Some(global) = global {
                self.image.globals[global] = value.ok_or(Trap::Memory)?;
            }
        }
        if let Some(global) = self.call_fields.time_global {
            self.image.globals[global] = (time.seconds() as f32).to_bits();
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
        rules: qa_core::primitives::RuleSetId,
        max_clients: i32,
    ) -> Result<Option<PhysicsEntity>, crate::services::CallError> {
        self.projection
            .quakec_physics_entity(entities, &self.entities, cursor, rules, max_clients)
    }
    /// Import bytecode field stores without replacing explicit native relinks.
    /// Scheduling remains owned by the provider's native think phase.
    pub fn project_entities(
        &mut self,
        services: &mut crate::services::EngineServices<'_>,
        context: crate::services::CallContext,
    ) -> Result<(), crate::services::CallError> {
        self.projection
            .quakec_project(services, &self.entities, &self.strings, context)?;
        self.dirty_entities.clear();
        self.dirty_count = 0;
        Ok(())
    }
    /// Spatial queries consume only successful bytecode entity writes since
    /// the previous import. Export projection also catches external byte edits.
    pub fn project_dirty_entities(
        &mut self,
        services: &mut crate::services::EngineServices<'_>,
        context: crate::services::CallContext,
    ) -> Result<(), crate::services::CallError> {
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
        self.entities.write_word(address, word as i32)?;
        // StoreP accepts native pointers, including an unaligned word crossing
        // a row boundary. Retain every successfully written row on later traps.
        let first = address as usize / self.stride;
        let last = (address as usize + 3) / self.stride;
        for slot in first..=last {
            if self.dirty_entities.insert(slot) {
                self.dirty_slots[self.dirty_count] = slot;
                self.dirty_count += 1;
            }
        }
        Ok(())
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
        if hooks {
            self.enter::<true>(builtins, function, budget)
        } else {
            self.enter::<false>(builtins, function, budget)
        }
    }
    fn enter<const HOOKS: bool>(
        &mut self,
        builtins: &mut impl Builtins,
        function: u32,
        budget: u64,
    ) -> Result<[u32; 3], Trap> {
        let saved = self.depth;
        let saved_statement = self.statement;
        let saved_argc = self.argc;
        let result = self.run::<HOOKS>(builtins, function, budget);
        while self.depth > saved {
            self.leave();
        }
        self.statement = saved_statement;
        self.argc = saved_argc;
        result
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
    fn run<const HOOKS: bool>(
        &mut self,
        builtins: &mut impl Builtins,
        function: u32,
        mut budget: u64,
    ) -> Result<[u32; 3], Trap> {
        use Opcode::*;
        let exit_depth = self.depth;
        let mut pc = self.push(function as usize, 0)?;
        loop {
            if budget <= 1 {
                return Err(Trap::Budget);
            }
            budget -= 1;
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
                    if self.depth == exit_depth {
                        return Ok([
                            self.image.globals[1],
                            self.image.globals[2],
                            self.image.globals[3],
                        ]);
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
                        builtins.call(self, f.first_statement.wrapping_neg() as u32, self.argc)?;
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
