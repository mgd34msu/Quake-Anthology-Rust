//! Numbered boundary entries. QVM and native pointers share these handlers.
use crate::{
    memory::ModuleMemory,
    quakec, qvm,
    services::{CallContext, CallError, ENGINE_CALLS, EngineServices},
};
use qa_core::{names::NameTable, primitives::PrintKind, sys_events::EventTime, text::FixedText};
use std::fmt::Write;

#[derive(Clone, Copy)]
pub enum Addresses {
    Qvm {
        mask: u32,
    },
    Native,
    /// Declared C function parameters, including full-width native size_t.
    NativeFunction,
}
pub struct Invocation<'a, 'engine, 'memory> {
    pub services: &'a mut EngineServices<'engine>,
    pub memory: &'a mut ModuleMemory<'memory>,
    pub native_cvars: Option<&'a mut crate::cvars::NativeCvars>,
    pub native_resources: Option<&'a [crate::services::ResourceRange; 3]>,
    pub native_entities: Option<(&'a mut crate::entities::EntityProjection, u64)>,
    pub context: CallContext,
    pub platform_time: EventTime,
    pub command: &'a [&'a [u8]],
    pub addresses: Addresses,
    pub arguments: &'a [u64],
}
impl Invocation<'_, '_, '_> {
    fn arg(&self, index: usize) -> Result<u64, CallError> {
        self.arguments.get(index).copied().ok_or(CallError::Memory)
    }
    fn pointer(&self, index: usize) -> Result<u64, CallError> {
        Ok(match self.addresses {
            Addresses::Qvm { mask } => u64::from(self.arg(index)? as u32 & mask),
            Addresses::Native | Addresses::NativeFunction => self.arg(index)?,
        })
    }
    fn length(&self, index: usize) -> Result<usize, CallError> {
        match self.addresses {
            Addresses::NativeFunction => {
                usize::try_from(self.arg(index)?).map_err(|_| CallError::Memory)
            }
            _ => usize::try_from(self.arg(index)? as u32 as i32).map_err(|_| CallError::Memory),
        }
    }
    fn string(&self, index: usize) -> Result<&[u8], CallError> {
        Ok(self.memory.cstring(self.pointer(index)?)?)
    }
    fn text(&self, index: usize) -> Result<&str, CallError> {
        std::str::from_utf8(self.string(index)?).map_err(|_| CallError::Text)
    }
}

type Entry = fn(&mut Invocation<'_, '_, '_>) -> Result<u64, CallError>;
pub struct CallTable {
    entries: [Option<Entry>; 256],
}
pub struct UnknownCalls {
    numbers: NameTable,
    pub calls: u64,
    pub capacity_drops: u64,
}
impl UnknownCalls {
    pub fn load(capacity: usize) -> Result<Self, qa_core::names::NamesError> {
        Ok(Self {
            numbers: NameTable::load_reserved(
                std::iter::empty(),
                capacity,
                capacity.saturating_mul(4),
            )?,
            calls: 0,
            capacity_drops: 0,
        })
    }
}
impl CallTable {
    pub fn invoke(
        &self,
        number: u32,
        call: &mut Invocation<'_, '_, '_>,
        unknown: &mut UnknownCalls,
    ) -> Result<u64, CallError> {
        if let Some(Some(entry)) = self.entries.get(number as usize) {
            return entry(call);
        }
        unknown.calls = unknown.calls.saturating_add(1);
        let key = number.to_le_bytes();
        if unknown.numbers.find(&key).is_none() {
            if unknown.numbers.intern(&key).is_ok() {
                let mut text = FixedText::<128>::default();
                writeln!(
                    text,
                    "module {}: unknown system call {number}",
                    call.context.module.0
                )
                .map_err(|_| CallError::Text)?;
                // A full output ring counts its loss; an unsupported import
                // still returns zero without aborting the module or engine.
                let _ =
                    (ENGINE_CALLS.print)(call.services, None, PrintKind::Console, text.as_bytes());
            } else {
                unknown.capacity_drops = unknown.capacity_drops.saturating_add(1);
            }
        }
        Ok(0)
    }
}

const fn common() -> CallTable {
    let mut table = CallTable {
        entries: [None; 256],
    };
    table.entries[100] = Some(memset);
    table.entries[101] = Some(memcpy);
    table.entries[102] = Some(strncpy);
    table.entries[103] = Some(sin::<false>);
    table.entries[104] = Some(cos::<false>);
    table.entries[105] = Some(atan2::<false>);
    table.entries[106] = Some(sqrt::<false>);
    table.entries[107] = Some(floor::<false>);
    table.entries[108] = Some(ceil::<false>);
    table
}
const fn server() -> CallTable {
    let mut t = common();
    t.entries[107] = None;
    t.entries[108] = None;
    t.entries[110] = Some(floor::<false>);
    t.entries[111] = Some(ceil::<false>);
    t.entries[0] = Some(print);
    t.entries[1] = Some(abort);
    t.entries[2] = Some(milliseconds);
    t.entries[5] = Some(cvar_set);
    t.entries[6] = Some(cvar_integer);
    t.entries[7] = Some(cvar_string);
    t.entries[8] = Some(argc);
    t.entries[9] = Some(argv);
    t.entries[10] = Some(file_open);
    t.entries[11] = Some(file_read);
    t.entries[13] = Some(file_close);
    t.entries[14] = Some(command);
    t.entries[18] = Some(config_set);
    t.entries[19] = Some(config_get);
    t
}
const fn client() -> CallTable {
    let mut t = common();
    t.entries[0] = Some(print);
    t.entries[1] = Some(abort);
    t.entries[2] = Some(milliseconds);
    t.entries[5] = Some(cvar_set);
    t.entries[6] = Some(cvar_string);
    t.entries[7] = Some(argc);
    t.entries[8] = Some(argv);
    t.entries[10] = Some(file_open);
    t.entries[11] = Some(file_read);
    t.entries[13] = Some(file_close);
    t.entries[14] = Some(command_append);
    t.entries[111] = Some(acos::<false>);
    t
}
const fn ui() -> CallTable {
    let mut t = common();
    t.entries[0] = Some(abort);
    t.entries[1] = Some(print);
    t.entries[2] = Some(milliseconds);
    t.entries[3] = Some(cvar_set);
    t.entries[4] = Some(cvar_number);
    t.entries[5] = Some(cvar_string);
    t.entries[10] = Some(argc);
    t.entries[11] = Some(argv);
    t.entries[13] = Some(file_open);
    t.entries[14] = Some(file_read);
    t.entries[16] = Some(file_close);
    t
}
// Original Q3 1.32 import ordinals. UI's ExecuteText and game's
// SendConsoleCommand have different argument layouts and are not aliases.
pub const Q3_SERVER: CallTable = server();
pub const Q3_CLIENT: CallTable = client();
pub const Q3_UI: CallTable = ui();

// Q2 function-pointer slots map into this same service table. Unimplemented
// native slots bind named traps before reaching this numbered dispatcher.
pub const Q2_CLASSIC: CallTable = {
    let mut table = CallTable {
        entries: [None; 256],
    };
    table.entries[6] = Some(config_set);
    table.entries[8] = Some(resource_index::<0>);
    table.entries[9] = Some(resource_index::<1>);
    table.entries[10] = Some(resource_index::<2>);
    table.entries[11] = Some(native_set_model);
    table.entries[18] = Some(native_link);
    table.entries[19] = Some(native_unlink);
    table.entries[36] = Some(q2_cvar);
    table.entries[37] = Some(q2_cvar_set::<false>);
    table.entries[38] = Some(q2_cvar_set::<true>);
    table
};
pub const Q2_RERELEASE: CallTable = {
    let mut table = CallTable {
        entries: [None; 256],
    };
    table.entries[1] = Some(print);
    table.entries[7] = Some(config_set);
    table.entries[10] = Some(resource_index::<0>);
    table.entries[11] = Some(resource_index::<1>);
    table.entries[12] = Some(resource_index::<2>);
    table.entries[13] = Some(native_set_model);
    table.entries[21] = Some(native_link);
    table.entries[22] = Some(native_unlink);
    table.entries[48] = Some(native_register_observer);
    table.entries[49] = Some(native_forget_observer);
    table.entries[9] = Some(abort);
    table.entries[39] = Some(q2_cvar);
    table.entries[40] = Some(q2_cvar_set::<false>);
    table.entries[41] = Some(q2_cvar_set::<true>);
    table
};

const fn quakec() -> CallTable {
    let mut t = CallTable {
        entries: [None; 256],
    };
    t.entries[25] = Some(qc_print);
    t.entries[37] = Some(floor::<false>);
    t.entries[38] = Some(ceil::<false>);
    t.entries[43] = Some(absolute::<false>);
    t.entries[45] = Some(cvar_number);
    t.entries[72] = Some(cvar_set);
    t
}
pub const QUAKEC: CallTable = quakec();

pub struct QuakeCCalls<'a, 'engine> {
    pub services: &'a mut EngineServices<'engine>,
    pub table: &'a CallTable,
    pub context: CallContext,
    pub platform_time: EventTime,
    pub developer: qa_core::primitives::CvarHandle,
    pub unknown: &'a mut UnknownCalls,
}
impl quakec::Builtins for QuakeCCalls<'_, '_> {
    fn call(&mut self, vm: &mut quakec::Vm, number: u32, argc: usize) -> Result<(), quakec::Trap> {
        // Unsupported native builtins trap only when executed; they never
        // inherit the Q3 convention of reporting an unknown import as zero.
        if self
            .table
            .entries
            .get(number as usize)
            .is_none_or(Option::is_none)
            || argc > 8
        {
            return Err(quakec::Trap::Builtin);
        }
        let expected = match number {
            25 => argc,
            72 => 2,
            _ => 1,
        };
        if argc != expected {
            return Err(quakec::Trap::Builtin);
        }
        let mut arguments = [0u64; 8];
        for (index, to) in arguments[..argc].iter_mut().enumerate() {
            let word = *vm
                .image
                .globals
                .get(4 + index * 3)
                .ok_or(quakec::Trap::Memory)?;
            if matches!(number, 25 | 45 | 72) {
                vm.string(word as i32)?;
            }
            *to = u64::from(word);
        }
        if number == 25 && self.services.cvars.value(self.developer) == 0.0 {
            return Ok(());
        }
        let mut invocation = Invocation {
            services: self.services,
            memory: &mut vm.strings,
            native_cvars: None,
            native_resources: None,
            native_entities: None,
            context: self.context,
            platform_time: self.platform_time,
            command: &[],
            addresses: Addresses::Native,
            arguments: &arguments[..argc],
        };
        let value = self
            .table
            .invoke(number, &mut invocation, self.unknown)
            .map_err(|_| quakec::Trap::Builtin)?;
        if !matches!(number, 25 | 72) {
            *vm.image.globals.get_mut(1).ok_or(quakec::Trap::Memory)? = value as u32;
        }
        Ok(())
    }
}

fn qc_print(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let mut text = FixedText::<1024>::default();
    for index in 0..c.arguments.len() {
        let bytes = c.memory.cstring(c.pointer(index)?)?;
        // QuakeC's VarString joins raw bytes rather than decoding UTF-8.
        text.append_bytes(bytes).map_err(|_| CallError::Text)?;
    }
    (ENGINE_CALLS.print)(c.services, None, PrintKind::Console, text.as_bytes())?;
    Ok(0)
}
fn absolute<const DOUBLE: bool>(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let value = c.arg(0)?;
    Ok(if DOUBLE {
        f64::from_bits(value).abs().to_bits()
    } else {
        u64::from(f32::from_bits(value as u32).abs().to_bits())
    })
}

pub struct QvmCalls<'a, 'engine> {
    pub services: &'a mut EngineServices<'engine>,
    pub table: &'a CallTable,
    pub context: CallContext,
    pub platform_time: EventTime,
    pub command: &'a [&'a [u8]],
    pub unknown: &'a mut UnknownCalls,
}
impl qvm::SystemCalls for QvmCalls<'_, '_> {
    fn call(&mut self, vm: &mut qvm::Vm, number: u32, arguments: &[i32]) -> Result<i32, qvm::Trap> {
        let mut native = [0u64; 15];
        let args = arguments.get(1..).ok_or(qvm::Trap::Syscall)?;
        if args.len() > native.len() {
            return Err(qvm::Trap::Syscall);
        }
        for (to, &from) in native.iter_mut().zip(args) {
            *to = u64::from(from as u32);
        }
        let mask = vm.memory.len() as u32 - 1;
        let mut call = Invocation {
            services: self.services,
            memory: &mut vm.memory,
            native_cvars: None,
            native_resources: None,
            native_entities: None,
            context: self.context,
            platform_time: self.platform_time,
            command: self.command,
            addresses: Addresses::Qvm { mask },
            arguments: &native[..args.len()],
        };
        self.table
            .invoke(number, &mut call, self.unknown)
            .map(|r| r as i32)
            .map_err(|_| qvm::Trap::Syscall)
    }
}

fn print(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let text = c.memory.cstring(c.pointer(0)?)?;
    (ENGINE_CALLS.print)(c.services, None, PrintKind::Console, text)?;
    Ok(0)
}
fn abort(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    print(c)?;
    Err(CallError::Aborted)
}
fn milliseconds(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    Ok(c.platform_time.milliseconds() as u32 as u64)
}
fn cvar_set(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let name = c.text(0)?;
    let view = c
        .services
        .cvars
        .bind(name, c.context.console)
        .ok_or(CallError::Cvar)?;
    let value =
        std::str::from_utf8(c.memory.cstring(c.pointer(1)?)?).map_err(|_| CallError::Text)?;
    (ENGINE_CALLS.cvar_set)(c.services, view, value)?;
    Ok(0)
}
fn q2_cvar(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let name_address = c.pointer(0)?;
    let default_address = c.pointer(1)?;
    let flags = c.arg(2)? as u32;
    let name = std::str::from_utf8(c.memory.cstring(name_address)?).map_err(|_| CallError::Text)?;
    let default = if default_address == 0 {
        None
    } else {
        Some(std::str::from_utf8(c.memory.cstring(default_address)?).map_err(|_| CallError::Text)?)
    };
    let view = match (ENGINE_CALLS.cvar_register)(
        c.services,
        c.context.console,
        name,
        default,
        crate::cvars::common_flags(flags),
    ) {
        Ok(view) => view,
        Err(CallError::Cvar | CallError::Capacity) => return Ok(0),
        Err(error) => return Err(error),
    };
    publish_cvar(c, view, flags)
}
fn q2_cvar_set<const FORCE: bool>(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let name_address = c.pointer(0)?;
    let value_address = c.pointer(1)?;
    let name = std::str::from_utf8(c.memory.cstring(name_address)?).map_err(|_| CallError::Text)?;
    let value =
        std::str::from_utf8(c.memory.cstring(value_address)?).map_err(|_| CallError::Text)?;
    let view = (ENGINE_CALLS.cvar_register)(c.services, c.context.console, name, Some(value), 0)?;
    let flags = c.services.cvars.flags(view);
    // Q2 NOSET applies even before the console's command-line INIT boundary.
    let result = if FORCE {
        (ENGINE_CALLS.cvar_force)(c.services, view, value)
    } else if flags & 16 != 0 {
        Err(CallError::Cvar)
    } else {
        (ENGINE_CALLS.cvar_set)(c.services, view, value)
    };
    if result == Err(CallError::Cvar) {
        let _ = (ENGINE_CALLS.print)(
            c.services,
            None,
            PrintKind::Console,
            b"cvar write rejected\n",
        );
    } else {
        result?;
    }
    publish_cvar(c, view, 0)
}
fn publish_cvar(
    c: &mut Invocation<'_, '_, '_>,
    view: qa_console::cvars::View,
    flags: u32,
) -> Result<u64, CallError> {
    match c.native_cvars.as_mut().ok_or(CallError::Cvar)?.publish(
        c.services.cvars,
        c.memory,
        view,
        flags,
    ) {
        Err(CallError::Capacity) => {
            let _ = (ENGINE_CALLS.print)(
                c.services,
                None,
                PrintKind::Console,
                b"native cvar capacity exceeded\n",
            );
            Ok(0)
        }
        result => result,
    }
}
fn cvar_integer(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let Some(view) = c.services.cvars.bind(c.text(0)?, c.context.console) else {
        return Ok(0);
    };
    let text = c.services.cvars.read(view).map_err(|_| CallError::Cvar)?;
    Ok(qa_console::numbers::integer(text.as_str()) as u32 as u64)
}
fn cvar_number(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let Some(view) = c.services.cvars.bind(c.text(0)?, c.context.console) else {
        return Ok(0);
    };
    Ok(c.services
        .cvars
        .numeric(view)
        .map_err(|_| CallError::Cvar)?
        .to_bits() as u64)
}
fn write_string(
    memory: &mut ModuleMemory<'_>,
    target: u64,
    length: usize,
    text: &[u8],
) -> Result<(), CallError> {
    let buffer = memory.read_mut(target, length)?;
    if length != 0 {
        let copied = text.len().min(length - 1);
        buffer[..copied].copy_from_slice(&text[..copied]);
        buffer[copied] = 0;
    }
    Ok(())
}
fn cvar_string(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let target = c.pointer(1)?;
    let length = c.length(2)?;
    let view = c.services.cvars.bind(c.text(0)?, c.context.console);
    if let Some(view) = view {
        let text = c.services.cvars.read(view).map_err(|_| CallError::Cvar)?;
        write_string(c.memory, target, length, text.as_str().as_bytes())?;
    } else {
        write_string(c.memory, target, length, b"")?;
    }
    Ok(0)
}
fn argc(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    Ok(c.command.len() as u64)
}
fn argv(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let index = c.arg(0)? as u32 as usize;
    let target = c.pointer(1)?;
    let length = c.length(2)?;
    write_string(
        c.memory,
        target,
        length,
        c.command.get(index).copied().unwrap_or(b""),
    )?;
    Ok(0)
}
fn command(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let text =
        std::str::from_utf8(c.memory.cstring(c.pointer(1)?)?).map_err(|_| CallError::Text)?;
    // Only EXEC_APPEND is implemented; immediate/insert execution must use the
    // existing command buffer's native ordering before those imports are enabled.
    if c.arg(0)? != 2 {
        return Err(CallError::Text);
    }
    (ENGINE_CALLS.command)(c.services, c.context.console, text)?;
    Ok(0)
}
fn command_append(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let text =
        std::str::from_utf8(c.memory.cstring(c.pointer(0)?)?).map_err(|_| CallError::Text)?;
    (ENGINE_CALLS.command)(c.services, c.context.console, text)?;
    Ok(0)
}
fn file_open(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    if c.arg(2)? != 0 {
        return Err(CallError::File);
    }
    if c.arg(1)? == 0 {
        return match (ENGINE_CALLS.file_length)(c.services, c.memory.cstring(c.pointer(0)?)?) {
            Ok(length) => Ok(length as u32 as u64),
            Err(CallError::File) => Ok(u32::MAX as u64),
            Err(error) => Err(error),
        };
    }
    let target = c.pointer(1)?;
    c.memory.read(target, 4)?;
    let result = (ENGINE_CALLS.file_open)(
        c.services,
        c.context.module,
        c.memory.cstring(c.pointer(0)?)?,
    );
    match result {
        Ok((handle, length)) => {
            c.memory.write_word(target, handle as i32)?;
            Ok(length as u32 as u64)
        }
        Err(CallError::File) => {
            c.memory.write_word(target, 0)?;
            Ok(u32::MAX as u64)
        }
        Err(error) => Err(error),
    }
}
fn file_read(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let target = c.pointer(0)?;
    let length = c.length(1)?;
    let handle = c.arg(2)? as u32;
    (ENGINE_CALLS.file_read)(
        c.services,
        c.context.module,
        handle,
        c.memory.read_mut(target, length)?,
    )?;
    Ok(0)
}
fn file_close(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    (ENGINE_CALLS.file_close)(c.services, c.context.module, c.arg(0)? as u32)?;
    Ok(0)
}
fn resource_index<const KIND: usize>(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let pointer = c.pointer(0)?;
    let range = c.native_resources.ok_or(CallError::ConfigString)?[KIND];
    let name = if pointer == 0 {
        &[]
    } else {
        c.memory.cstring(pointer)?
    };
    (ENGINE_CALLS.resource_index)(c.services, c.context.module, range, name).map(u64::from)
}
fn native_set_model(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let address = c.pointer(0)?;
    let name = c.pointer(1)?;
    let (entities, table) = c.native_entities.as_mut().ok_or(CallError::Entity)?;
    entities.set_model(c.services, c.memory, c.context, *table, address, name)?;
    Ok(0)
}
fn native_link(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let address = c.pointer(0)?;
    let (entities, table) = c.native_entities.as_mut().ok_or(CallError::Entity)?;
    entities.link(c.services, c.memory, c.context, *table, address)?;
    Ok(0)
}
fn native_unlink(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let address = c.pointer(0)?;
    let (entities, table) = c.native_entities.as_mut().ok_or(CallError::Entity)?;
    entities.unlink(c.services, c.memory, c.context, *table, address)?;
    Ok(0)
}
fn native_forget_observer(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let address = c.pointer(0)?;
    let (entities, table) = c.native_entities.as_mut().ok_or(CallError::Entity)?;
    entities.forget_observer(c.services, c.memory, c.context, *table, address)?;
    Ok(0)
}
fn native_register_observer(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let address = c.pointer(0)?;
    let (entities, table) = c.native_entities.as_mut().ok_or(CallError::Entity)?;
    entities.register_observer(c.services, c.memory, c.context, *table, address)?;
    Ok(0)
}
fn config_set(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let pointer = c.pointer(1)?;
    (ENGINE_CALLS.configstring)(
        c.services,
        c.context.module,
        c.length(0)?,
        if pointer == 0 {
            &[]
        } else {
            c.memory.cstring(pointer)?
        },
    )?;
    Ok(0)
}
fn config_get(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let target = c.pointer(1)?;
    let length = c.length(2)?;
    let (text, _) = c
        .services
        .storage
        .configstring(c.context.module, c.length(0)?)?;
    write_string(c.memory, target, length, text)?;
    Ok(0)
}
fn memset(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let target = c.pointer(0)?;
    let value = c.arg(1)? as u8;
    let length = c.length(2)?;
    c.memory.read_mut(target, length)?.fill(value);
    Ok(if matches!(c.addresses, Addresses::NativeFunction) {
        target
    } else {
        0
    })
}
fn memcpy(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    c.memory.copy(c.pointer(0)?, c.pointer(1)?, c.length(2)?)?;
    Ok(if matches!(c.addresses, Addresses::NativeFunction) {
        c.pointer(0)?
    } else {
        0
    })
}
fn strncpy(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let from = c.pointer(1)?;
    let to = c.pointer(0)?;
    let length = c.length(2)?;
    // Validate both extents before writes; strncpy may read an unterminated
    // source when its first n bytes fit. Padding starts at the first zero.
    c.memory.read(to, length)?;
    let mut zero = false;
    for i in 0..length {
        let byte = if zero {
            0
        } else {
            c.memory.read(from + i as u64, 1)?[0]
        };
        zero |= byte == 0;
        c.memory.write(to + i as u64, &[byte])?;
    }
    c.arg(0)
}
fn float<const DOUBLE: bool>(c: &Invocation<'_, '_, '_>, index: usize) -> Result<f64, CallError> {
    let value = c.arg(index)?;
    Ok(if DOUBLE {
        f64::from_bits(value)
    } else {
        f32::from_bits(value as u32) as f64
    })
}
fn bits<const DOUBLE: bool>(value: f64) -> u64 {
    if DOUBLE {
        value.to_bits()
    } else {
        u64::from((value as f32).to_bits())
    }
}
fn sin<const DOUBLE: bool>(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    Ok(bits::<DOUBLE>(float::<DOUBLE>(c, 0)?.sin()))
}
fn cos<const DOUBLE: bool>(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    Ok(bits::<DOUBLE>(float::<DOUBLE>(c, 0)?.cos()))
}
fn atan2<const DOUBLE: bool>(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    Ok(bits::<DOUBLE>(
        float::<DOUBLE>(c, 0)?.atan2(float::<DOUBLE>(c, 1)?),
    ))
}
fn sqrt<const DOUBLE: bool>(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    Ok(bits::<DOUBLE>(float::<DOUBLE>(c, 0)?.sqrt()))
}
fn floor<const DOUBLE: bool>(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    Ok(bits::<DOUBLE>(float::<DOUBLE>(c, 0)?.floor()))
}
fn ceil<const DOUBLE: bool>(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    Ok(bits::<DOUBLE>(float::<DOUBLE>(c, 0)?.ceil()))
}
fn acos<const DOUBLE: bool>(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    Ok(bits::<DOUBLE>(float::<DOUBLE>(c, 0)?.acos()))
}
