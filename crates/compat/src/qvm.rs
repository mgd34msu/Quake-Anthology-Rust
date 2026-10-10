//! One safe hot interpreter, preserving qsrc's native stack and byte-PC ABI.
use crate::memory::ModuleMemory;
use qa_formats::program::qvm::{Image, Opcode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trap {
    Memory,
    OperandStack,
    ProgramStack,
    Target,
    Division,
    Budget,
    Syscall,
    Return,
}
impl From<crate::memory::MemoryError> for Trap {
    fn from(_: crate::memory::MemoryError) -> Self {
        Self::Memory
    }
}

pub trait SystemCalls {
    fn call(&mut self, vm: &mut Vm, number: u32, arguments: &[i32]) -> Result<i32, Trap>;
}
pub use crate::hooks::Hooks;

pub struct Vm {
    pub memory: ModuleMemory<'static>,
    pub image: Image,
    pub hooks: Hooks,
    program_stack: usize,
    stack_bottom: usize,
    mask: u32,
}
impl Vm {
    pub fn load(image: Image) -> Result<Self, Trap> {
        let memory = ModuleMemory::load(0, image.memory_size, &image.initialized)?;
        let program_stack = memory.len();
        Ok(Self {
            mask: (memory.len() - 1) as u32,
            memory,
            image,
            hooks: Hooks::load(program_stack),
            program_stack,
            stack_bottom: program_stack.saturating_sub(65536),
        })
    }
    pub fn masked_address(&self, address: i32) -> u64 {
        u64::from(address as u32 & self.mask)
    }
    pub fn call(
        &mut self,
        calls: &mut impl SystemCalls,
        arguments: [i32; 10],
        budget: u64,
        hooks: bool,
    ) -> Result<i32, Trap> {
        // Selection happens once. Const specialization removes every hook from
        // the ordinary loop, without maintaining a second opcode implementation.
        if hooks {
            self.enter::<true>(calls, arguments, budget)
        } else {
            self.enter::<false>(calls, arguments, budget)
        }
    }
    fn enter<const HOOKS: bool>(
        &mut self,
        calls: &mut impl SystemCalls,
        arguments: [i32; 10],
        budget: u64,
    ) -> Result<i32, Trap> {
        let saved_stack = self.program_stack;
        let result = self.run::<HOOKS>(calls, arguments, budget);
        // A failed call, including reentry, restores the native caller's stack.
        self.program_stack = saved_stack;
        result
    }
    fn run<const HOOKS: bool>(
        &mut self,
        calls: &mut impl SystemCalls,
        arguments: [i32; 10],
        budget: u64,
    ) -> Result<i32, Trap> {
        use Opcode::*;
        let mut stack = [0i32; 256];
        let mut depth = 0usize;
        let mut pc = 0usize;
        let mut program_stack = self
            .program_stack
            .checked_sub(48)
            .filter(|&s| s >= self.stack_bottom)
            .ok_or(Trap::ProgramStack)?;
        self.memory.write_word(program_stack as u64, -1)?;
        self.memory.write_word(program_stack as u64 + 4, 0)?;
        for (index, arg) in arguments.into_iter().enumerate() {
            self.memory
                .write_word((program_stack + 8 + index * 4) as u64, arg)?;
        }
        for _ in 0..budget {
            let instruction = *self.image.instructions.get(pc).ok_or(Trap::Target)?;
            pc += 1;
            let op = instruction.opcode;
            let operand = instruction.operand;
            if HOOKS {
                self.hooks.instructions += 1;
            }
            match op {
                Ignore | Break => {}
                Enter => {
                    program_stack = program_stack
                        .checked_sub(operand as usize)
                        .filter(|&s| s >= self.stack_bottom)
                        .ok_or(Trap::ProgramStack)?;
                }
                Leave => {
                    program_stack = program_stack
                        .checked_add(operand as usize)
                        .filter(|&s| s < self.memory.len())
                        .ok_or(Trap::ProgramStack)?;
                    let target = self.memory.read_word(program_stack as u64)?;
                    if target == -1 {
                        return if depth == 1 {
                            Ok(stack[1])
                        } else {
                            Err(Trap::Return)
                        };
                    }
                    let ordinal = self
                        .image
                        .byte_to_instruction
                        .get(target as usize)
                        .copied()
                        .filter(|&i| i != u32::MAX)
                        .ok_or(Trap::Target)?;
                    pc = ordinal as usize;
                }
                Const | Local | Push => {
                    depth += 1;
                    let slot = stack.get_mut(depth).ok_or(Trap::OperandStack)?;
                    *slot = match op {
                        Const => operand,
                        Local => (program_stack as i32).wrapping_add(operand),
                        _ => 0,
                    };
                }
                Pop => {
                    depth = depth.checked_sub(1).ok_or(Trap::OperandStack)?;
                }
                Jump | Call => {
                    if depth == 0 {
                        return Err(Trap::OperandStack);
                    }
                    let target = stack[depth];
                    depth -= 1;
                    if op == Call {
                        if HOOKS {
                            self.hooks.calls += 1;
                        }
                        self.memory
                            .write_word(program_stack as u64, instruction.next_byte_pc as i32)?;
                        if target < 0 {
                            let number = (-1i32).wrapping_sub(target) as u32;
                            self.memory
                                .write_word(program_stack as u64 + 4, number as i32)?;
                            let mut arguments = [0; 16];
                            arguments[0] = number as i32;
                            let available =
                                ((self.memory.len() - program_stack - 4) / 4).min(arguments.len());
                            for (index, arg) in
                                arguments.iter_mut().take(available).enumerate().skip(1)
                            {
                                *arg = self
                                    .memory
                                    .read_word((program_stack + 4 + index * 4) as u64)?;
                            }
                            self.program_stack =
                                program_stack.checked_sub(4).ok_or(Trap::ProgramStack)?;
                            let result = calls.call(self, number, &arguments[..available])?;
                            depth += 1;
                            *stack.get_mut(depth).ok_or(Trap::OperandStack)? = result;
                            let target = self.memory.read_word(program_stack as u64)?;
                            pc = self
                                .image
                                .byte_to_instruction
                                .get(target as usize)
                                .copied()
                                .filter(|&i| i != u32::MAX)
                                .ok_or(Trap::Target)? as usize;
                            continue;
                        }
                    }
                    pc = usize::try_from(target)
                        .ok()
                        .filter(|&i| i < self.image.instructions.len())
                        .ok_or(Trap::Target)?;
                }
                Load1 | Load2 | Load4 => {
                    if depth == 0 {
                        return Err(Trap::OperandStack);
                    }
                    let address = self.masked_address(stack[depth]);
                    stack[depth] = match op {
                        Load1 => i32::from(self.memory.read(address, 1)?[0]),
                        Load2 => {
                            let b = self.memory.read(address, 2)?;
                            i32::from(u16::from_le_bytes([b[0], b[1]]))
                        }
                        _ => self.memory.read_word(address)?,
                    };
                }
                Store1 | Store2 | Store4 => {
                    if depth < 2 {
                        return Err(Trap::OperandStack);
                    }
                    let width = match op {
                        Store1 => 1,
                        Store2 => 2,
                        _ => 4,
                    };
                    let address = (stack[depth - 1] as u32 & self.mask & !(width - 1)) as u64;
                    self.memory
                        .write(address, &stack[depth].to_le_bytes()[..width as usize])?;
                    depth -= 2;
                    if HOOKS {
                        self.hooks.stores += 1;
                        self.hooks.write(address as usize, width as usize);
                    }
                }
                Arg => {
                    if depth == 0 {
                        return Err(Trap::OperandStack);
                    }
                    self.memory
                        .write_word((program_stack + operand as usize) as u64, stack[depth])?;
                    depth -= 1;
                    if HOOKS {
                        self.hooks.stores += 1;
                        self.hooks.write(program_stack + operand as usize, 4);
                    }
                }
                BlockCopy => {
                    if depth < 2 {
                        return Err(Trap::OperandStack);
                    }
                    let from = stack[depth] as u32 & self.mask;
                    let to = stack[depth - 1] as u32 & self.mask;
                    let count =
                        ((from.wrapping_add(operand as u32) & self.mask) as i64) - i64::from(from);
                    let count =
                        ((to.wrapping_add(count as u32) & self.mask) as i64) - i64::from(to);
                    if (from | to | count as u32) & 3 != 0 {
                        return Err(Trap::Memory);
                    }
                    let words = (count / 4).max(0) as usize;
                    for i in (0..words).rev() {
                        let word = self.memory.read_word(u64::from(from) + i as u64 * 4)?;
                        self.memory.write_word(u64::from(to) + i as u64 * 4, word)?;
                    }
                    depth -= 2;
                    if HOOKS {
                        self.hooks.stores += words as u64;
                        self.hooks.write(to as usize, words * 4);
                    }
                }
                Eq | Ne | LtI | LeI | GtI | GeI | LtU | LeU | GtU | GeU | EqF | NeF | LtF | LeF
                | GtF | GeF => {
                    if depth < 2 {
                        return Err(Trap::OperandStack);
                    }
                    let a = stack[depth - 1];
                    let b = stack[depth];
                    depth -= 2;
                    let branch = match op {
                        Eq => a == b,
                        Ne => a != b,
                        LtI => a < b,
                        LeI => a <= b,
                        GtI => a > b,
                        GeI => a >= b,
                        LtU => (a as u32) < b as u32,
                        LeU => (a as u32) <= b as u32,
                        GtU => (a as u32) > b as u32,
                        GeU => (a as u32) >= b as u32,
                        EqF => f32::from_bits(a as u32) == f32::from_bits(b as u32),
                        NeF => f32::from_bits(a as u32) != f32::from_bits(b as u32),
                        LtF => f32::from_bits(a as u32) < f32::from_bits(b as u32),
                        LeF => f32::from_bits(a as u32) <= f32::from_bits(b as u32),
                        GtF => f32::from_bits(a as u32) > f32::from_bits(b as u32),
                        GeF => f32::from_bits(a as u32) >= f32::from_bits(b as u32),
                        _ => false,
                    };
                    if branch {
                        pc = operand as usize;
                    }
                }
                Sex8 | Sex16 | NegI | NegF | CvIF | CvFI => {
                    if depth == 0 {
                        return Err(Trap::OperandStack);
                    }
                    let a = stack[depth];
                    stack[depth] = match op {
                        Sex8 => i32::from(a as i8),
                        Sex16 => i32::from(a as i16),
                        NegI => a.wrapping_neg(),
                        NegF => (-f32::from_bits(a as u32)).to_bits() as i32,
                        CvIF => (a as f32).to_bits() as i32,
                        _ => crate::numbers::native_integer(f32::from_bits(a as u32)),
                    };
                }
                Bcom => {
                    // qsrc's interpreted ABI writes the previous operand, not
                    // the top operand. Compiled ABI lowering is separate work.
                    if depth == 0 {
                        return Err(Trap::OperandStack);
                    }
                    stack[depth - 1] = !stack[depth];
                }
                Add | Sub | DivI | DivU | ModI | ModU | MulI | MulU | Band | Bor | Bxor | Lsh
                | RshI | RshU | AddF | SubF | DivF | MulF => {
                    if depth < 2 {
                        return Err(Trap::OperandStack);
                    }
                    let a = stack[depth - 1];
                    let b = stack[depth];
                    depth -= 1;
                    stack[depth] =
                        match op {
                            Add => a.wrapping_add(b),
                            Sub => a.wrapping_sub(b),
                            MulI | MulU => a.wrapping_mul(b),
                            DivI => a.checked_div(b).ok_or(Trap::Division)?,
                            ModI => a.checked_rem(b).ok_or(Trap::Division)?,
                            DivU => (a as u32).checked_div(b as u32).ok_or(Trap::Division)? as i32,
                            ModU => (a as u32).checked_rem(b as u32).ok_or(Trap::Division)? as i32,
                            Band => a & b,
                            Bor => a | b,
                            Bxor => a ^ b,
                            Lsh => a.wrapping_shl(b as u32),
                            RshI => a.wrapping_shr(b as u32),
                            RshU => (a as u32).wrapping_shr(b as u32) as i32,
                            AddF => (f32::from_bits(a as u32) + f32::from_bits(b as u32)).to_bits()
                                as i32,
                            SubF => (f32::from_bits(a as u32) - f32::from_bits(b as u32)).to_bits()
                                as i32,
                            DivF => (f32::from_bits(a as u32) / f32::from_bits(b as u32)).to_bits()
                                as i32,
                            MulF => (f32::from_bits(a as u32) * f32::from_bits(b as u32)).to_bits()
                                as i32,
                            _ => return Err(Trap::Target),
                        };
                }
            }
        }
        Err(Trap::Budget)
    }
}
