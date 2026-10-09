//! One safe version-six execution loop with optional hooks selected per call.
use crate::numbers::native_integer;
use crate::{
    hooks::Hooks,
    memory::{MemoryError, ModuleMemory},
};
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
    self_global: usize,
    time_global: usize,
    nextthink: usize,
    frame: usize,
    think: usize,
}
pub struct Vm {
    pub image: Image,
    pub entities: ModuleMemory,
    pub strings: ModuleMemory,
    pub hooks: Hooks,
    pub active: bool,
    pub argc: usize,
    pub statement: usize,
    pub stride: usize,
    pub header_bytes: usize,
    state: Option<StateFields>,
    state_step: f64,
    stack: [Frame; 32],
    locals: [u32; 2048],
    depth: usize,
    locals_used: usize,
    string_used: usize,
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
        let extent = stride
            .checked_mul(layout.entities)
            .filter(|&n| n <= i32::MAX as usize)
            .ok_or(Trap::Memory)?;
        let entities = ModuleMemory::load(0, extent, &[])?;
        let string_used = image.strings.len();
        let strings = ModuleMemory::load(
            0,
            string_used
                .checked_add(layout.extra_string_bytes)
                .ok_or(Trap::Memory)?,
            &image.strings,
        )?;
        let state = (|| {
            Some(StateFields {
                self_global: image.global(b"self")?,
                time_global: image.global(b"time")?,
                nextthink: image.field(b"nextthink")?,
                frame: image.field(b"frame")?,
                think: image.field(b"think")?,
            })
        })();
        let hook_bytes = image
            .globals
            .len()
            .checked_mul(4)
            .and_then(|b| b.checked_add(extent))
            .ok_or(Trap::Memory)?;
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
            state,
            state_step: layout.state_step,
            stack: [Frame::default(); 32],
            locals: [0; 2048],
            depth: 0,
            locals_used: 0,
            string_used,
        })
    }
    pub fn string(&self, offset: i32) -> Result<&[u8], Trap> {
        let address = u64::try_from(offset).map_err(|_| Trap::String)?;
        if address >= self.string_used as u64 {
            return Err(Trap::String);
        }
        let tail = self
            .strings
            .read(address, self.string_used - address as usize)?;
        Ok(&tail[..tail.iter().position(|&b| b == 0).ok_or(Trap::String)?])
    }
    pub fn insert_string(&mut self, text: &[u8]) -> Result<i32, Trap> {
        let end = self
            .string_used
            .checked_add(text.len())
            .and_then(|n| n.checked_add(1))
            .filter(|&n| n <= i32::MAX as usize && n <= self.strings.len())
            .ok_or(Trap::String)?;
        let address = self.string_used;
        self.strings.write(address as u64, text)?;
        self.strings.write((end - 1) as u64, &[0])?;
        self.string_used = end;
        Ok(address as i32)
    }
    /// Native entity values are byte offsets into this backing, never common
    /// EntityIds or client ordinals. Engine views read these same bytes.
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
                        self.entities
                            .write_word(address + i as u64 * 4, self.image.globals[a + i] as i32)?;
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
                    let entity = self.image.globals[state.self_global];
                    let nextthink = self.field_address(entity, state.nextthink as u32, 1)?;
                    let frame = self.field_address(entity, state.frame as u32, 1)?;
                    let think = self.field_address(entity, state.think as u32, 1)?;
                    let time = f32::from_bits(self.image.globals[state.time_global]);
                    self.entities.write_word(
                        nextthink,
                        ((time as f64 + self.state_step) as f32).to_bits() as i32,
                    )?;
                    if f32::from_bits(self.image.globals[a])
                        != f32::from_bits(self.entities.read_word(frame)? as u32)
                    {
                        self.entities
                            .write_word(frame, self.image.globals[a] as i32)?;
                    }
                    self.entities
                        .write_word(think, self.image.globals[b] as i32)?;
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
                        MulV => av[0] * bv[0] + av[1] * bv[1] + av[2] * bv[2],
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
