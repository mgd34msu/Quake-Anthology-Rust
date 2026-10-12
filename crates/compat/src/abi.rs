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
    Native {
        abi: qa_platform::native::NativeAbi,
        pointer_bytes: u8,
    },
    /// Declared C function parameters, including full-width native size_t.
    NativeFunction {
        abi: qa_platform::native::NativeAbi,
        pointer_bytes: u8,
    },
}
pub struct Invocation<'a, 'engine, 'memory> {
    pub services: &'a mut EngineServices<'engine>,
    pub memory: &'a mut ModuleMemory<'memory>,
    pub native_cvars: Option<&'a mut crate::cvars::NativeCvars>,
    pub native_resources: Option<&'a [qa_core::primitives::IndexSpan; 3]>,
    pub native_entities: Option<&'a mut crate::entities::EntityProjection>,
    pub native_surfaces: Option<&'a crate::surfaces::NativeSurfaces>,
    pub native_command: Option<&'a crate::command::NativeCommand>,
    pub native_configs: Option<&'a mut crate::configstrings::NativeConfigs>,
    pub native_pmove: Option<&'a mut crate::pmove::Pmove>,
    pub native_player: Option<qa_core::primitives::ClientId>,
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
            Addresses::Native { .. } | Addresses::NativeFunction { .. } => self.arg(index)?,
        })
    }
    fn null_pointer(&self, index: usize) -> Result<bool, CallError> {
        // VM_ArgPtr tests the argument before applying dataMask. A nonzero
        // QVM address that wraps to offset zero still names actual data.
        Ok(match self.addresses {
            Addresses::Qvm { .. } => self.arg(index)? as u32 == 0,
            Addresses::Native { .. } | Addresses::NativeFunction { .. } => self.arg(index)? == 0,
        })
    }
    fn required_pointer(&self, index: usize) -> Result<u64, CallError> {
        if self.null_pointer(index)? {
            return Err(CallError::Memory);
        }
        self.pointer(index)
    }
    fn vector(&self, index: usize, nullable: bool) -> Result<qa_core::primitives::Vec3, CallError> {
        if nullable && self.null_pointer(index)? {
            return Ok(qa_core::primitives::Vec3::default());
        }
        let vector = self.memory.read_vec3(self.required_pointer(index)?)?;
        vector
            .is_finite()
            .then_some(vector)
            .ok_or(CallError::Geometry)
    }
    fn bounds(
        &self,
        first: usize,
        nullable: bool,
    ) -> Result<qa_core::primitives::Bounds, CallError> {
        let bounds = qa_core::primitives::Bounds {
            mins: self.vector(first, nullable)?,
            maxs: self.vector(first + 1, nullable)?,
        };
        bounds
            .is_valid()
            .then_some(bounds)
            .ok_or(CallError::Geometry)
    }
    fn length(&self, index: usize) -> Result<usize, CallError> {
        match self.addresses {
            Addresses::NativeFunction { .. } => {
                usize::try_from(self.arg(index)?).map_err(|_| CallError::Memory)
            }
            _ => usize::try_from(self.arg(index)? as u32 as i32).map_err(|_| CallError::Memory),
        }
    }
    fn string(&self, index: usize) -> Result<&[u8], CallError> {
        Ok(self.memory.cstring(self.pointer(index)?)?)
    }
    fn text(&self, index: usize) -> Result<&str, CallError> {
        text_at(self.memory, self.pointer(index)?)
    }
}

fn text_at<'a>(memory: &'a ModuleMemory<'_>, address: u64) -> Result<&'a str, CallError> {
    std::str::from_utf8(memory.cstring(address)?).map_err(|_| CallError::Text)
}

enum CallValue {
    Word(u64),
    TemporaryString(u32, u8),
    Vector(qa_core::primitives::Vec3),
    Basis(qa_core::math::AngleBasis),
}
#[derive(Clone, Copy)]
enum QcArgc {
    Fixed(usize),
    Variadic,
}
#[derive(Clone, Copy)]
enum QcArgs {
    Scalars,
    Vector,
    Strings {
        from: usize,
        qw_from: usize,
    },
    ClientStrings {
        from: usize,
        qw_from: usize,
    },
    /// Parameter widths remain native parm0+n*3; the flat service arguments
    /// contain only each declared scalar/vector component.
    Mixed {
        vectors: u8,
        strings: u8,
    },
}
impl QcArgs {
    fn string_start(self, quakeworld: bool) -> Option<usize> {
        match self {
            Self::Strings { from, qw_from } | Self::ClientStrings { from, qw_from } => {
                Some(if quakeworld { qw_from } else { from })
            }
            _ => None,
        }
    }
    fn width(self, argument: usize) -> usize {
        let vector = match self {
            Self::Vector => argument == 0,
            Self::Mixed { vectors, .. } => vectors & (1 << argument) != 0,
            _ => false,
        };
        if vector { 3 } else { 1 }
    }
    fn string(self, argument: usize, first: Option<usize>) -> bool {
        match self {
            Self::Mixed { strings, .. } => strings & (1 << argument) != 0,
            _ => first.is_some_and(|first| argument >= first),
        }
    }
}
#[derive(Clone, Copy)]
struct QcSignature {
    argc: QcArgc,
    args: QcArgs,
    returns: bool,
    developer_nq_only: bool,
}
impl QcSignature {
    const fn new(argc: QcArgc, args: QcArgs, returns: bool) -> Self {
        Self {
            argc,
            args,
            returns,
            developer_nq_only: false,
        }
    }
    const fn void(mut self) -> Self {
        self.returns = false;
        self
    }
    const fn developer(mut self) -> Self {
        self.developer_nq_only = true;
        self
    }
}
type Entry = fn(&mut Invocation<'_, '_, '_>, Option<usize>) -> Result<CallValue, CallError>;
type QcEntry =
    fn(&mut QuakeCCalls<'_, '_>, &mut quakec::Vm, &[u64]) -> Result<CallValue, CallError>;
#[derive(Clone, Copy)]
enum Handler {
    Shared(Entry),
    QuakeC(QcEntry),
}
#[derive(Clone, Copy)]
struct CallEntry {
    handler: Handler,
    quakec: Option<QcSignature>,
}
impl CallEntry {
    fn invoke(
        self,
        call: &mut Invocation<'_, '_, '_>,
        string_start: Option<usize>,
    ) -> Result<CallValue, CallError> {
        match self.handler {
            Handler::Shared(entry) => entry(call, string_start),
            Handler::QuakeC(_) => Err(CallError::Memory),
        }
    }
}
macro_rules! scalar {
    ($handler:expr) => {
        scalar!($handler, None)
    };
    ($handler:expr, $signature:expr) => {
        Some(CallEntry {
            handler: Handler::Shared(|call, _| ($handler)(call).map(CallValue::Word)),
            quakec: $signature,
        })
    };
}
macro_rules! value {
    ($handler:expr, $signature:expr) => {
        Some(CallEntry {
            handler: Handler::Shared(|call, _| ($handler)(call)),
            quakec: Some($signature),
        })
    };
}
macro_rules! qc {
    ($handler:expr, $argc:expr, $vectors:expr, $returns:expr) => {
        Some(CallEntry {
            handler: Handler::QuakeC($handler),
            quakec: Some(QcSignature::new(
                QcArgc::Fixed($argc),
                QcArgs::Mixed {
                    vectors: $vectors,
                    strings: 0,
                },
                $returns,
            )),
        })
    };
}
pub struct CallTable {
    entries: [Option<CallEntry>; 256],
}
pub struct UnknownCalls {
    numbers: NameTable,
    pub calls: u64,
    pub capacity_drops: u64,
    pub last: Option<u32>,
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
            last: None,
        })
    }
    fn report(
        &mut self,
        services: &mut EngineServices<'_>,
        module: qa_core::primitives::ModuleId,
        number: u32,
        kind: &str,
    ) {
        self.calls = self.calls.saturating_add(1);
        self.last = Some(number);
        let key = number.to_le_bytes();
        if self.numbers.find(&key).is_none() {
            if self.numbers.intern(&key).is_ok() {
                let _ = (ENGINE_CALLS.console_formatted)(
                    services,
                    format_args!("module {}: unknown {kind} {number}\n", module.0),
                );
            } else {
                self.capacity_drops = self.capacity_drops.saturating_add(1);
            }
        }
    }
}
impl CallTable {
    pub fn invoke(
        &self,
        number: u32,
        call: &mut Invocation<'_, '_, '_>,
        unknown: &mut UnknownCalls,
    ) -> Result<u64, CallError> {
        match self.invoke_value(number, call, unknown)? {
            CallValue::Word(value) => Ok(value),
            CallValue::Vector(_) | CallValue::Basis(_) | CallValue::TemporaryString(..) => {
                Err(CallError::Memory)
            }
        }
    }
    fn invoke_value(
        &self,
        number: u32,
        call: &mut Invocation<'_, '_, '_>,
        unknown: &mut UnknownCalls,
    ) -> Result<CallValue, CallError> {
        if let Some(Some(entry)) = self.entries.get(number as usize) {
            return entry.invoke(call, None);
        }
        // Q3 returns zero for an unsupported import; a full output ring
        // still counts its loss through the common publisher.
        unknown.report(call.services, call.context.module, number, "system call");
        Ok(CallValue::Word(0))
    }
}

const fn common() -> CallTable {
    let mut table = CallTable {
        entries: [None; 256],
    };
    table.entries[100] = scalar!(memset);
    table.entries[101] = scalar!(memcpy);
    table.entries[102] = scalar!(strncpy);
    table.entries[103] = scalar!(sin::<false>);
    table.entries[104] = scalar!(cos::<false>);
    table.entries[105] = scalar!(atan2::<false>);
    table.entries[106] = scalar!(sqrt::<false>);
    table.entries[107] = scalar!(floor::<false>);
    table.entries[108] = scalar!(ceil::<false>);
    table
}
const fn server() -> CallTable {
    let mut t = common();
    t.entries[107] = None;
    t.entries[108] = None;
    t.entries[110] = scalar!(floor::<false>);
    t.entries[111] = scalar!(ceil::<false>);
    t.entries[0] = scalar!(print);
    t.entries[1] = scalar!(abort);
    t.entries[2] = scalar!(milliseconds);
    t.entries[3] = scalar!(q3_cvar_register);
    t.entries[4] = scalar!(q3_cvar_update);
    t.entries[5] = scalar!(cvar_set::<true, true>);
    t.entries[6] = scalar!(cvar_number::<true>);
    t.entries[7] = scalar!(cvar_string);
    t.entries[8] = scalar!(argc);
    t.entries[9] = scalar!(argv);
    t.entries[10] = scalar!(file_open);
    t.entries[11] = scalar!(file_read);
    t.entries[13] = scalar!(file_close);
    t.entries[14] = scalar!(command);
    t.entries[15] = scalar!(q3_locate_game_data);
    t.entries[30] = scalar!(native_link);
    t.entries[31] = scalar!(native_unlink);
    t.entries[18] = scalar!(config_set);
    t.entries[19] = scalar!(config_get);
    t.entries[23] = scalar!(q3_set_brush_model);
    t.entries[24] = scalar!(q3_trace);
    t.entries[25] = scalar!(q3_point_contents);
    t.entries[26] = scalar!(q3_visibility::<false>);
    t.entries[27] = scalar!(q3_visibility::<true>);
    t.entries[29] = scalar!(q3_areas_connected);
    t.entries[32] = scalar!(q3_entities_in_box);
    t.entries[33] = scalar!(q3_entity_contact);
    t.entries[36] = scalar!(q3_usercmd);
    t.entries[37] = scalar!(entity_token);
    t.entries[42] = scalar!(snap_vector);
    t.entries[43] = scalar!(q3_capsule_unsupported);
    t.entries[44] = scalar!(q3_capsule_unsupported);
    t.entries[45] = scalar!(file_seek);
    t
}
const fn client() -> CallTable {
    let mut t = common();
    t.entries[0] = scalar!(print);
    t.entries[1] = scalar!(abort);
    t.entries[2] = scalar!(milliseconds);
    t.entries[3] = scalar!(q3_cvar_register);
    t.entries[4] = scalar!(q3_cvar_update);
    t.entries[5] = scalar!(cvar_set::<true, true>);
    t.entries[6] = scalar!(cvar_string);
    t.entries[7] = scalar!(argc);
    t.entries[8] = scalar!(argv);
    t.entries[9] = scalar!(client_args);
    t.entries[10] = scalar!(file_open);
    t.entries[11] = scalar!(file_read);
    t.entries[13] = scalar!(file_close);
    t.entries[14] = scalar!(command_append);
    t.entries[53] = scalar!(client_server_command);
    t.entries[56] = scalar!(q3_set_usercmd_value);
    t.entries[71] = scalar!(snap_vector);
    t.entries[89] = scalar!(file_seek);
    t.entries[111] = scalar!(acos::<false>);
    t
}
const fn ui() -> CallTable {
    let mut t = common();
    t.entries[0] = scalar!(abort);
    t.entries[1] = scalar!(print);
    t.entries[2] = scalar!(milliseconds);
    t.entries[3] = scalar!(cvar_set::<true, true>);
    t.entries[4] = scalar!(cvar_number::<false>);
    t.entries[5] = scalar!(cvar_string);
    t.entries[6] = scalar!(cvar_set_value);
    t.entries[7] = scalar!(cvar_reset);
    t.entries[8] = scalar!(cvar_create);
    t.entries[10] = scalar!(argc);
    t.entries[11] = scalar!(argv);
    t.entries[12] = scalar!(command);
    t.entries[13] = scalar!(file_open);
    t.entries[14] = scalar!(file_read);
    t.entries[16] = scalar!(file_close);
    t.entries[86] = scalar!(file_seek);
    t.entries[50] = scalar!(q3_cvar_register);
    t.entries[51] = scalar!(q3_cvar_update);
    t
}
// Original Q3 1.32 import ordinals. Game/UI commands carry an execution mode;
// cgame's SendConsoleCommand accepts only text and appends it.
pub const Q3_SERVER: CallTable = server();
pub const Q3_CLIENT: CallTable = client();
pub const Q3_UI: CallTable = ui();

// Q2 function-pointer slots map into this same service table. Unimplemented
// native slots bind named traps before reaching this numbered dispatcher.
pub(crate) mod q2;
pub use q2::{CLASSIC as Q2_CLASSIC, RERELEASE as Q2_RERELEASE};

const fn quakec() -> CallTable {
    let mut t = CallTable {
        entries: [None; 256],
    };
    let vector = QcSignature::new(QcArgc::Fixed(1), QcArgs::Vector, true);
    let scalar = QcSignature::new(QcArgc::Fixed(1), QcArgs::Scalars, true);
    let strings = QcSignature::new(
        QcArgc::Fixed(1),
        QcArgs::Strings {
            from: 0,
            qw_from: 0,
        },
        true,
    );
    t.entries[1] = value!(angle_basis, vector.void());
    t.entries[2] = qc!(qc_set_origin, 2, 2, false);
    t.entries[3] = Some(CallEntry {
        handler: Handler::QuakeC(qc_set_model),
        quakec: Some(QcSignature::new(
            QcArgc::Fixed(2),
            QcArgs::Mixed {
                vectors: 0,
                strings: 2,
            },
            false,
        )),
    });
    t.entries[4] = qc!(qc_set_size, 3, 6, false);
    t.entries[8] = Some(CallEntry {
        handler: Handler::QuakeC(qc_sound),
        quakec: Some(QcSignature::new(
            QcArgc::Fixed(5),
            QcArgs::Mixed {
                vectors: 0,
                strings: 4,
            },
            false,
        )),
    });
    t.entries[9] = value!(vector_normalize, vector);
    t.entries[12] = scalar!(vector_length, Some(vector));
    t.entries[13] = scalar!(vector_yaw, Some(vector));
    t.entries[14] = qc!(qc_spawn, 0, 0, true);
    t.entries[15] = qc!(qc_remove, 1, 0, false);
    t.entries[16] = qc!(qc_trace_line, 4, 3, false);
    t.entries[18] = Some(CallEntry {
        handler: Handler::QuakeC(qc_find),
        quakec: Some(QcSignature::new(
            QcArgc::Fixed(3),
            QcArgs::Mixed {
                vectors: 0,
                strings: 4,
            },
            true,
        )),
    });
    t.entries[19] = Some(CallEntry {
        handler: Handler::QuakeC(qc_precache::<false>),
        quakec: Some(strings),
    });
    t.entries[20] = Some(CallEntry {
        handler: Handler::QuakeC(qc_precache::<true>),
        quakec: Some(strings),
    });
    t.entries[23] = Some(CallEntry {
        handler: Handler::Shared(qc_print::<true>),
        quakec: Some(QcSignature::new(
            QcArgc::Variadic,
            QcArgs::Strings {
                from: 0,
                qw_from: 1,
            },
            false,
        )),
    });
    t.entries[25] = Some(CallEntry {
        handler: Handler::Shared(qc_print::<false>),
        quakec: Some(QcSignature::new(QcArgc::Variadic, strings.args, false).developer()),
    });
    t.entries[24] = Some(CallEntry {
        handler: Handler::Shared(qc_client_print::<false>),
        quakec: Some(QcSignature::new(
            QcArgc::Variadic,
            QcArgs::ClientStrings {
                from: 1,
                qw_from: 2,
            },
            false,
        )),
    });
    t.entries[73] = Some(CallEntry {
        handler: Handler::Shared(qc_client_print::<true>),
        quakec: Some(QcSignature::new(
            QcArgc::Variadic,
            QcArgs::ClientStrings {
                from: 1,
                qw_from: 1,
            },
            false,
        )),
    });
    t.entries[26] = value!(qc_float_string, scalar);
    t.entries[27] = value!(qc_vector_string, vector);
    t.entries[34] = qc!(qc_drop_to_floor, 0, 0, true);
    t.entries[35] = scalar!(
        lightstyle,
        Some(QcSignature::new(
            QcArgc::Fixed(2),
            QcArgs::Mixed {
                vectors: 0,
                strings: 2,
            },
            false,
        ))
    );
    t.entries[36] = scalar!(rint, Some(scalar));
    t.entries[37] = scalar!(floor::<false>, Some(scalar));
    t.entries[38] = scalar!(ceil::<false>, Some(scalar));
    t.entries[41] = qc!(qc_point_contents, 1, 1, true);
    t.entries[43] = scalar!(absolute::<false>, Some(scalar));
    t.entries[45] = scalar!(cvar_number::<false>, Some(strings));
    t.entries[46] = scalar!(command_append, Some(strings.void()));
    t.entries[51] = value!(vector_angles, vector);
    t.entries[68] = Some(CallEntry {
        handler: Handler::QuakeC(qc_precache_file),
        quakec: Some(strings),
    });
    t.entries[69] = qc!(qc_make_static, 1, 0, true);
    t.entries[74] = Some(CallEntry {
        handler: Handler::QuakeC(qc_ambient_sound),
        quakec: Some(QcSignature::new(
            QcArgc::Fixed(4),
            QcArgs::Mixed {
                vectors: 1,
                strings: 2,
            },
            true,
        )),
    });
    t.entries[75] = t.entries[20];
    t.entries[76] = t.entries[19];
    t.entries[77] = t.entries[68];
    t.entries[72] = scalar!(
        cvar_set::<false, false>,
        Some(QcSignature {
            argc: QcArgc::Fixed(2),
            ..strings.void()
        })
    );
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
        let Some(entry) = self.table.entries.get(number as usize).copied().flatten() else {
            self.unknown
                .report(self.services, self.context.module, number, "QuakeC builtin");
            return Err(quakec::Trap::Builtin);
        };
        let signature = entry.quakec.ok_or(quakec::Trap::Builtin)?;
        if argc > 8 {
            return Err(quakec::Trap::Builtin);
        }
        if matches!(signature.argc, QcArgc::Fixed(expected) if argc != expected) {
            return Err(quakec::Trap::Builtin);
        }
        let mut arguments = [0u64; 24];
        let mut words = 0;
        let quakeworld = self.context.native_rules == qa_core::primitives::RuleSetId::QuakeWorld;
        let string_start = signature.args.string_start(quakeworld);
        for index in 0..argc {
            for component in 0..signature.args.width(index) {
                let mut word = *vm
                    .image
                    .globals
                    .get(4 + index * 3 + component)
                    .ok_or(quakec::Trap::Memory)?;
                if component == 0 && signature.args.string(index, string_start) {
                    vm.string(word as i32)?;
                }
                if index == 0 && matches!(signature.args, QcArgs::ClientStrings { .. }) {
                    word = vm.entity_number(word)?;
                }
                arguments[words] = u64::from(word);
                words += 1;
            }
        }
        if signature.developer_nq_only
            && !quakeworld
            && self.services.cvars.value(self.developer) == 0.0
        {
            return Ok(());
        }
        let value = if let Handler::QuakeC(entry) = entry.handler {
            entry(self, vm, &arguments[..words]).map_err(|_| quakec::Trap::Builtin)?
        } else {
            let mut invocation = Invocation {
                services: self.services,
                memory: &mut vm.strings,
                native_cvars: None,
                native_resources: None,
                native_entities: None,
                native_surfaces: None,
                native_command: None,
                native_configs: None,
                native_pmove: None,
                native_player: None,
                context: self.context,
                platform_time: self.platform_time,
                command: &[],
                addresses: Addresses::Native {
                    abi: qa_platform::native::NativeAbi::SystemV,
                    pointer_bytes: 8,
                },
                arguments: &arguments[..words],
            };
            entry
                .invoke(&mut invocation, string_start)
                .map_err(|_| quakec::Trap::Builtin)?
        };
        match value {
            CallValue::Word(value) => {
                if signature.returns {
                    *vm.image.globals.get_mut(1).ok_or(quakec::Trap::Memory)? = value as u32;
                }
            }
            CallValue::TemporaryString(offset, length) => {
                vm.register_temporary_string(length);
                if signature.returns {
                    *vm.image.globals.get_mut(1).ok_or(quakec::Trap::Memory)? = offset;
                }
            }
            CallValue::Vector(vector) => {
                if signature.returns {
                    vm.write_global_vector(1, vector)?;
                }
            }
            CallValue::Basis(basis) => vm.write_basis(basis)?,
        }
        Ok(())
    }
}

fn qc_word(arguments: &[u64], index: usize) -> Result<u32, CallError> {
    arguments
        .get(index)
        .copied()
        .map(|word| word as u32)
        .ok_or(CallError::Memory)
}
fn qc_vector(arguments: &[u64], first: usize) -> Result<qa_core::primitives::Vec3, CallError> {
    Ok(qa_core::primitives::Vec3([
        f32::from_bits(qc_word(arguments, first)?),
        f32::from_bits(qc_word(arguments, first + 1)?),
        f32::from_bits(qc_word(arguments, first + 2)?),
    ]))
}
fn qc_spawn(
    c: &mut QuakeCCalls<'_, '_>,
    vm: &mut quakec::Vm,
    _: &[u64],
) -> Result<CallValue, CallError> {
    Ok(CallValue::Word(u64::from(
        vm.spawn_entity(c.services, c.context)?,
    )))
}
fn qc_remove(
    c: &mut QuakeCCalls<'_, '_>,
    vm: &mut quakec::Vm,
    arguments: &[u64],
) -> Result<CallValue, CallError> {
    vm.remove_entity(c.services, c.context, qc_word(arguments, 0)?)?;
    Ok(CallValue::Word(0))
}
fn qc_find(
    c: &mut QuakeCCalls<'_, '_>,
    vm: &mut quakec::Vm,
    arguments: &[u64],
) -> Result<CallValue, CallError> {
    Ok(CallValue::Word(u64::from(vm.find_entity(
        &c.services.server.entities,
        c.context.module,
        qc_word(arguments, 0)?,
        qc_word(arguments, 1)?,
        qc_word(arguments, 2)? as i32,
    )?)))
}
fn qc_set_origin(
    c: &mut QuakeCCalls<'_, '_>,
    vm: &mut quakec::Vm,
    arguments: &[u64],
) -> Result<CallValue, CallError> {
    vm.projection.quakec_set_origin(
        c.services,
        &mut vm.entities,
        &vm.strings,
        c.context,
        qc_word(arguments, 0)?,
        qc_vector(arguments, 1)?,
    )?;
    Ok(CallValue::Word(0))
}
fn qc_drop_to_floor(
    c: &mut QuakeCCalls<'_, '_>,
    vm: &mut quakec::Vm,
    _: &[u64],
) -> Result<CallValue, CallError> {
    Ok(CallValue::Word(u64::from(
        f32::from(u8::from(vm.drop_to_floor(c.services, c.context)?)).to_bits(),
    )))
}
fn qc_set_size(
    c: &mut QuakeCCalls<'_, '_>,
    vm: &mut quakec::Vm,
    arguments: &[u64],
) -> Result<CallValue, CallError> {
    let bounds = qa_core::primitives::Bounds {
        mins: qc_vector(arguments, 1)?,
        maxs: qc_vector(arguments, 4)?,
    };
    vm.projection.quakec_set_size(
        c.services,
        &mut vm.entities,
        &vm.strings,
        c.context,
        qc_word(arguments, 0)?,
        bounds,
    )?;
    Ok(CallValue::Word(0))
}
fn qc_set_model(
    c: &mut QuakeCCalls<'_, '_>,
    vm: &mut quakec::Vm,
    arguments: &[u64],
) -> Result<CallValue, CallError> {
    vm.projection.quakec_set_model(
        c.services,
        &mut vm.entities,
        &vm.strings,
        c.context,
        qc_word(arguments, 0)?,
        qc_word(arguments, 1)?,
    )?;
    Ok(CallValue::Word(0))
}
fn lightstyle(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let value = float::<false>(c, 0)?;
    if !value.is_finite() {
        return Err(CallError::ConfigString);
    }
    let style = crate::numbers::native_integer(value);
    let ordinal = usize::try_from(style).map_err(|_| CallError::ConfigString)?;
    let text = c.memory.cstring(c.pointer(1)?)?;
    (ENGINE_CALLS.lightstyle)(c.services, c.context.module, ordinal, text)?;
    Ok(0)
}
fn qc_precache<const MODEL: bool>(
    c: &mut QuakeCCalls<'_, '_>,
    vm: &mut quakec::Vm,
    arguments: &[u64],
) -> Result<CallValue, CallError> {
    if vm.active {
        return Err(CallError::Aborted);
    }
    let word = qc_word(arguments, 0)?;
    let name = vm.strings.cstring(u64::from(word))?;
    if name.first().is_none_or(|&first| first <= b' ') {
        return Err(CallError::Text);
    }
    let resources = c
        .services
        .storage
        .resources(c.context.module)
        .ok_or(CallError::ConfigString)?;
    let (_, inserted) = (ENGINE_CALLS.resource_register)(
        c.services,
        c.context.module,
        resources[usize::from(!MODEL)],
        name,
    )?;
    // QW registers the name only. NQ Mod_ForName runs after the new native
    // precache row has been written, so a failed load retains that row.
    if MODEL && inserted && c.context.native_rules != qa_core::primitives::RuleSetId::QuakeWorld {
        if name.starts_with(b"*") {
            crate::entities::inline_model(
                c.services.geometry,
                c.services.world.ok_or(CallError::Geometry)?.0,
                name,
            )?;
        } else {
            (ENGINE_CALLS.model_precache)(c.services, name, c.context.native_rules)?;
        }
    }
    Ok(CallValue::Word(u64::from(word)))
}
fn qc_precache_file(
    _: &mut QuakeCCalls<'_, '_>,
    _: &mut quakec::Vm,
    arguments: &[u64],
) -> Result<CallValue, CallError> {
    Ok(CallValue::Word(u64::from(qc_word(arguments, 0)?)))
}
fn qc_precached_sound(
    c: &mut QuakeCCalls<'_, '_>,
    name: &[u8],
    prefix: &[u8],
    suffix: &[u8],
) -> Result<Option<(qa_core::primitives::IndexSpan, u32)>, CallError> {
    let sounds = c
        .services
        .storage
        .resources(c.context.module)
        .ok_or(CallError::ConfigString)?[1];
    if let Some(ordinal) = (ENGINE_CALLS.resource_find)(c.services, c.context.module, sounds, name)?
    {
        return Ok(Some((sounds, ordinal)));
    }
    let mut text = qa_core::text::FixedText::<8240>::default();
    text.append_bytes(prefix).map_err(|_| CallError::Capacity)?;
    text.append_bytes(name).map_err(|_| CallError::Capacity)?;
    text.append_bytes(suffix).map_err(|_| CallError::Capacity)?;
    match (ENGINE_CALLS.console)(c.services, text.as_bytes()) {
        Ok(_) | Err(CallError::Capacity) => Ok(None),
        Err(error) => Err(error),
    }
}
fn qc_make_static(
    c: &mut QuakeCCalls<'_, '_>,
    vm: &mut quakec::Vm,
    arguments: &[u64],
) -> Result<CallValue, CallError> {
    vm.make_static(c.services, c.context, qc_word(arguments, 0)?)?;
    Ok(CallValue::Word(0))
}
fn qc_ambient_sound(
    c: &mut QuakeCCalls<'_, '_>,
    vm: &mut quakec::Vm,
    arguments: &[u64],
) -> Result<CallValue, CallError> {
    let name = vm.strings.cstring(u64::from(qc_word(arguments, 3)?))?;
    let Some((_, source_ordinal)) = qc_precached_sound(c, name, b"no precache: ", b"\n")? else {
        return Ok(CallValue::Word(0));
    };
    let result = (ENGINE_CALLS.ambient)(
        c.services,
        c.context,
        crate::services::AmbientSound {
            source_ordinal,
            origin: qc_vector(arguments, 0)?
                .0
                .map(qa_core::math::eighth_to_short),
            volume: crate::numbers::native_integer(f64::from(
                f32::from_bits(qc_word(arguments, 4)?) * 255.0,
            )) as u8,
            attenuation: crate::numbers::native_integer(f64::from(
                f32::from_bits(qc_word(arguments, 5)?) * 64.0,
            )) as u8,
        },
    );
    if let Err(error) = result
        && error != CallError::Capacity
    {
        return Err(error);
    }
    Ok(CallValue::Word(0))
}
fn qc_sound(
    c: &mut QuakeCCalls<'_, '_>,
    vm: &mut quakec::Vm,
    arguments: &[u64],
) -> Result<CallValue, CallError> {
    use qa_core::primitives::{NativeDelivery, NativeTarget, RuleSetId, SoundEmitter, SoundEvent};
    let rules = c.context.native_rules;
    let channel = crate::numbers::native_integer(f64::from(f32::from_bits(qc_word(arguments, 1)?)));
    let volume =
        crate::numbers::native_integer(f64::from(f32::from_bits(qc_word(arguments, 3)?) * 255.0));
    let attenuation = f32::from_bits(qc_word(arguments, 4)?);
    let maximum_channel = if rules == RuleSetId::QuakeWorld {
        15
    } else {
        7
    };
    if !(0..=maximum_channel).contains(&channel)
        || !(0..=255).contains(&volume)
        || !(0.0..=4.0).contains(&attenuation)
    {
        return Err(CallError::Text);
    }
    let name = vm.strings.cstring(u64::from(qc_word(arguments, 2)?))?;
    let Some((sounds, ordinal)) =
        qc_precached_sound(c, name, b"SV_StartSound: ", b" not precached\n")?
    else {
        return Ok(CallValue::Word(0));
    };
    let Some(sound) =
        (ENGINE_CALLS.sound_id)(c.services, c.context.module, sounds, ordinal as i32)?
    else {
        return Ok(CallValue::Word(0));
    };
    let (entity, position, positioned) = vm.projection.quakec_sound_source(
        c.services,
        &vm.entities,
        &vm.strings,
        c.context,
        qc_word(arguments, 0)?,
    )?;
    let hearing = rules == RuleSetId::QuakeWorld
        && channel & 8 == 0
        && c.services.storage.uses_sound_phs(c.services.cvars);
    let reliable = rules == RuleSetId::QuakeWorld && channel & 8 != 0;
    let delivery = NativeDelivery {
        module: c.context.module,
        source: rules,
        target: if hearing {
            NativeTarget::Hearing
        } else {
            NativeTarget::All
        },
        origin: position,
        reliable,
        dupe_key: 0,
        component: 0,
        emission: None,
        check_portals: false,
    };
    let event = SoundEvent {
        sound,
        source_ordinal: Some(ordinal),
        spatial: c.context.sound_policy,
        entity: Some(SoundEmitter::Entity(entity)),
        position,
        channel: (channel & 7) as u16,
        volume: volume as f32 / 255.0,
        attenuation,
        positioned,
        ..SoundEvent::default()
    };
    match (ENGINE_CALLS.sound)(c.services, delivery, event) {
        Ok(_) | Err(CallError::Capacity) => Ok(CallValue::Word(0)),
        Err(error) => Err(error),
    }
}
fn qc_trace_line(
    c: &mut QuakeCCalls<'_, '_>,
    vm: &mut quakec::Vm,
    arguments: &[u64],
) -> Result<CallValue, CallError> {
    use qa_world::collision::{QuakeTraceKind, TraceQuery, trace_policy};
    let (rules, entity_rules) = trace_policy(c.context.trace_rules);
    let kind =
        match crate::numbers::native_integer(f64::from(f32::from_bits(qc_word(arguments, 6)?))) {
            1 => QuakeTraceKind::IgnoreBoxes,
            2 => QuakeTraceKind::Missile,
            _ => QuakeTraceKind::Normal,
        };
    let mut query = TraceQuery::point(
        qc_vector(arguments, 0)?,
        qc_vector(arguments, 3)?,
        rules,
        entity_rules.with_quake_kind(Some(kind)),
    );
    let (view, pass) = vm.projection.quakec_trace_view(
        &vm.entities,
        &vm.strings,
        c.context,
        qc_word(arguments, 7)?,
    )?;
    query.pass = pass;
    let trace = (ENGINE_CALLS.trace)(c.services, query, Some(&view))?;
    view.finish()?;
    let entity = vm.projection.quakec_hit(c.services, trace.entity);
    vm.write_trace(trace, entity)
        .map_err(|_| CallError::Memory)?;
    Ok(CallValue::Word(0))
}

fn argument_vector(c: &Invocation<'_, '_, '_>) -> Result<qa_core::primitives::Vec3, CallError> {
    Ok(qa_core::primitives::Vec3([
        float::<false>(c, 0)? as f32,
        float::<false>(c, 1)? as f32,
        float::<false>(c, 2)? as f32,
    ]))
}
fn vector_length(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    Ok(bits::<false>(f64::from(qa_core::math::length(
        argument_vector(c)?,
    ))))
}
fn vector_normalize(c: &mut Invocation<'_, '_, '_>) -> Result<CallValue, CallError> {
    Ok(CallValue::Vector(qa_core::math::normalized_or_zero(
        argument_vector(c)?,
    )))
}
fn angle_basis(c: &mut Invocation<'_, '_, '_>) -> Result<CallValue, CallError> {
    Ok(CallValue::Basis(qa_core::math::angle_vectors(
        argument_vector(c)?,
    )))
}
fn native_direction_angle(vertical: f32, horizontal: f32) -> f32 {
    let mut angle = crate::numbers::native_integer(
        f64::from(vertical).atan2(f64::from(horizontal)) * 180.0 / std::f64::consts::PI,
    ) as f32;
    if angle < 0.0 {
        angle += 360.0;
    }
    angle
}
fn vector_yaw(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let [x, y, _] = argument_vector(c)?.0;
    let yaw = if x == 0.0 && y == 0.0 {
        0.0
    } else {
        native_direction_angle(y, x)
    };
    Ok(bits::<false>(f64::from(yaw)))
}
fn vector_angles(c: &mut Invocation<'_, '_, '_>) -> Result<CallValue, CallError> {
    let [x, y, z] = argument_vector(c)?.0;
    let (pitch, yaw) = if x == 0.0 && y == 0.0 {
        (if z > 0.0 { 90.0 } else { 270.0 }, 0.0)
    } else {
        let forward = qa_core::math::length(qa_core::primitives::Vec3([x, y, 0.0]));
        (
            native_direction_angle(z, forward),
            native_direction_angle(y, x),
        )
    };
    Ok(CallValue::Vector(qa_core::primitives::Vec3([
        pitch, yaw, 0.0,
    ])))
}
fn qc_print<const BROADCAST: bool>(
    c: &mut Invocation<'_, '_, '_>,
    first: Option<usize>,
) -> Result<CallValue, CallError> {
    let first = first.ok_or(CallError::Memory)?;
    let level = qc_print_level(c, first, 0)?;
    let text = qc_var_string(c, first)?;
    (ENGINE_CALLS.print)(
        c.services,
        if BROADCAST {
            None
        } else {
            Some(qa_core::events::OutputTarget::Presentation)
        },
        None,
        if BROADCAST {
            PrintKind::from_level(level)
        } else {
            PrintKind::Console
        },
        level,
        text.as_bytes(),
    )?;
    Ok(CallValue::Word(0))
}
fn qc_temporary(
    c: &mut Invocation<'_, '_, '_>,
    text: &FixedText<{ quakec::TEMP_STRING_BYTES }>,
) -> Result<CallValue, CallError> {
    let offset =
        quakec::temporary_string(c.memory, text.as_bytes()).map_err(|_| CallError::Text)?;
    Ok(CallValue::TemporaryString(
        offset,
        text.as_bytes().len() as u8,
    ))
}
fn qc_float_string(c: &mut Invocation<'_, '_, '_>) -> Result<CallValue, CallError> {
    let value = float::<false>(c, 0)?;
    let mut text = FixedText::<{ quakec::TEMP_STRING_BYTES }>::default();
    crate::numbers::native_number(&mut text, value, 5, 1).map_err(|_| CallError::Text)?;
    qc_temporary(c, &text)
}
fn qc_vector_string(c: &mut Invocation<'_, '_, '_>) -> Result<CallValue, CallError> {
    let mut text = FixedText::<{ quakec::TEMP_STRING_BYTES }>::default();
    text.write_char('\'').map_err(|_| CallError::Text)?;
    for index in 0..3 {
        if index != 0 {
            text.write_char(' ').map_err(|_| CallError::Text)?;
        }
        crate::numbers::fixed_float(&mut text, float::<false>(c, index)?, 5, 1)
            .map_err(|_| CallError::Text)?;
    }
    text.write_char('\'').map_err(|_| CallError::Text)?;
    qc_temporary(c, &text)
}

fn qc_var_string(c: &Invocation<'_, '_, '_>, first: usize) -> Result<FixedText<1024>, CallError> {
    let mut text = FixedText::<1024>::default();
    for index in first..c.arguments.len() {
        let bytes = c.memory.cstring(c.pointer(index)?)?;
        // QuakeC's VarString joins raw bytes rather than decoding UTF-8.
        text.append_bytes(bytes).map_err(|_| CallError::Text)?;
    }
    Ok(text)
}
fn qc_print_level(c: &Invocation<'_, '_, '_>, first: usize, base: usize) -> Result<u8, CallError> {
    Ok(if first > base {
        crate::numbers::native_integer(float::<false>(c, base)?) as u8
    } else {
        PrintKind::Console.default_level()
    })
}
fn qc_client_print<const CENTER: bool>(
    c: &mut Invocation<'_, '_, '_>,
    first: Option<usize>,
) -> Result<CallValue, CallError> {
    let first = first.ok_or(CallError::Memory)?;
    let text = qc_var_string(c, first)?;
    let native = qa_core::primitives::NativeEntity {
        module: c.context.module,
        slot: i32::try_from(c.arg(0)?).map_err(|_| CallError::Entity)?,
    };
    let Some(client) = c
        .services
        .server
        .client_for_native(native)
        .filter(|_| native.slot > 0)
    else {
        (ENGINE_CALLS.console)(c.services, b"tried to sprint to a non-client\n")?;
        return Ok(CallValue::Word(0));
    };
    let level = qc_print_level(c, first, 1)?;
    (ENGINE_CALLS.print)(
        c.services,
        None,
        Some(client),
        if CENTER {
            PrintKind::Center
        } else {
            PrintKind::from_level(level)
        },
        level,
        text.as_bytes(),
    )?;
    Ok(CallValue::Word(0))
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
    fn begin_call(&mut self, vm: &mut qvm::Vm) -> Result<(), qvm::Trap> {
        if let Some(projection) = &mut vm.projection {
            projection
                .seed(&self.services.server.entities, self.context.module)
                .map_err(|_| qvm::Trap::Syscall)?;
        }
        Ok(())
    }
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
            native_entities: vm.projection.as_mut().map(|p| &mut p.entities),
            native_surfaces: None,
            native_command: None,
            native_configs: None,
            native_pmove: None,
            native_player: None,
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
    (ENGINE_CALLS.console)(c.services, text)?;
    Ok(0)
}
struct PrintRoute {
    target: Option<qa_core::events::OutputTarget>,
    client: Option<qa_core::primitives::ClientId>,
    kind: PrintKind,
}
fn print_route(
    c: &Invocation<'_, '_, '_>,
    address: u64,
    unicast: bool,
    level: u8,
) -> Result<Option<PrintRoute>, CallError> {
    let client = if unicast && address != 0 {
        let Some(client) = native_client(c, address)? else {
            // A free client or non-client edict never becomes a broadcast.
            return Ok(None);
        };
        Some(client)
    } else {
        None
    };
    let local = unicast && address == 0;
    Ok(Some(PrintRoute {
        client,
        target: local.then_some(qa_core::events::OutputTarget::Presentation),
        kind: if local {
            PrintKind::Console
        } else {
            PrintKind::from_localized_level(level)
        },
    }))
}
fn native_print<const TARGET: bool, const CENTER: bool>(
    c: &mut Invocation<'_, '_, '_>,
) -> Result<u64, CallError> {
    let address = if TARGET { c.pointer(0)? } else { 0 };
    if CENTER && address == 0 {
        return Ok(0);
    }
    let level = if CENTER {
        PrintKind::Center.default_level()
    } else {
        c.arg(usize::from(TARGET))? as u8
    };
    let Some(route) = print_route(c, address, TARGET, level)? else {
        return Ok(0);
    };
    let text = c
        .memory
        .cstring(c.pointer(usize::from(TARGET) + usize::from(!CENTER))?)?;
    (ENGINE_CALLS.print)(
        c.services,
        route.target,
        route.client,
        if CENTER {
            PrintKind::Center
        } else {
            route.kind
        },
        level,
        text,
    )?;
    Ok(0)
}
fn native_localized_print(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let address = c.pointer(0)?;
    let flags = c.arg(1)? as u8;
    let broadcast = flags & 8 != 0;
    let mut level = flags & !(8 | 16);
    if broadcast && level > 3 && level != 6 {
        level = 3;
    }
    let Some(route) = print_route(c, address, !broadcast, level)? else {
        return Ok(0);
    };
    let base = c.memory.cstring(c.pointer(2)?)?;
    let count = usize::try_from(c.arg(4)?).map_err(|_| CallError::Memory)?;
    let words = if count == 0 {
        &[][..]
    } else {
        c.memory.read(
            c.pointer(3)?,
            count.checked_mul(8).ok_or(CallError::Memory)?,
        )?
    };
    let argument = |index: usize| {
        let bytes = &words[index * 8..index * 8 + 8];
        let pointer = u64::from_le_bytes(bytes.try_into().map_err(|_| CallError::Memory)?);
        if pointer == 0 {
            Ok(None)
        } else {
            c.memory.cstring(pointer).map(Some).map_err(CallError::from)
        }
    };
    (ENGINE_CALLS.localized_print)(
        c.services,
        route.target,
        route.client,
        route.kind,
        level,
        base,
        count,
        &argument,
    )?;
    Ok(0)
}
fn native_sound<const POSITIONED: bool, const LOCAL: bool, const WIDE: bool>(
    c: &mut Invocation<'_, '_, '_>,
) -> Result<u64, CallError> {
    use qa_core::primitives::{NativeTarget, SoundAction, SoundEvent, Vec3};
    let address;
    let client = if LOCAL {
        let target = c.pointer(0)?;
        if target == 0 {
            return Ok(0);
        }
        let Some(client) = native_client(c, target)? else {
            return Ok(0);
        };
        address = c.pointer(2)?;
        Some(client)
    } else {
        address = c.pointer(usize::from(POSITIONED))?;
        if address == 0 && !POSITIONED {
            return Ok(0);
        }
        None
    };
    let index = if LOCAL {
        4
    } else {
        2 + usize::from(POSITIONED)
    };
    let channel = c.arg(index - 1)? as u16;
    let volume = float::<false>(c, index + 1)? as f32;
    let attenuation = float::<false>(c, index + 2)? as f32;
    let time_offset = float::<false>(c, index + 3)? as f32;
    if !(0.0..=1.0).contains(&volume)
        || !(0.0..=4.0).contains(&attenuation)
        || !(0.0..=0.255).contains(&time_offset)
    {
        return Err(CallError::Text);
    }
    let resources = c.native_resources.ok_or(CallError::Text)?;
    let Some(sound) = (ENGINE_CALLS.sound_id)(
        c.services,
        c.context.module,
        resources[1],
        c.arg(index)? as i32,
    )?
    else {
        return Ok(0);
    };
    let origin = if POSITIONED {
        c.pointer(usize::from(LOCAL))?
    } else {
        0
    };
    let (entity, mut position, mut positioned) = if address == 0 {
        if origin == 0 {
            return Ok(0);
        }
        (None, Vec3::default(), true)
    } else {
        let entities = c.native_entities.as_mut().ok_or(CallError::Entity)?;
        let (entity, position, positioned) =
            entities.sound_source(c.services, c.memory, c.context, address)?;
        (Some(entity), position, positioned)
    };
    if origin != 0 {
        position = c.memory.read_vec3(origin)?;
        positioned = true;
    }
    let dupe_key = if LOCAL { c.arg(8)? as u32 } else { 0 };
    let target = if let Some(client) = client {
        NativeTarget::Client(client)
    } else if channel & 8 != 0 || attenuation == 0.0 {
        NativeTarget::All
    } else {
        NativeTarget::Hearing
    };
    // Classic SV_StartSound masks the whole channel for NO_PHS; rerelease
    // preserves the independent reliable bit.
    let reliable = channel & 16 != 0 && (WIDE || channel & 8 == 0);
    positioned |= WIDE && reliable;
    let delivery = native_delivery::<WIDE>(c, target, position, reliable, dupe_key);
    match (ENGINE_CALLS.sound)(
        c.services,
        delivery,
        SoundEvent {
            looping: None,
            sound,
            source_ordinal: Some(c.arg(index)? as u32),
            native_fields: None,
            spatial: c.context.sound_policy,
            entity,
            position,
            client: None,
            client_lifetime: None,
            volume,
            attenuation,
            time_offset,
            channel: channel & 7,
            positioned,
            action: SoundAction::Play,
        },
    ) {
        Ok(_) | Err(CallError::Capacity) => Ok(0),
        Err(error) => Err(error),
    }
}
fn abort(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    print(c)?;
    Err(CallError::Aborted)
}
fn milliseconds(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    Ok(c.platform_time.milliseconds() as u32 as u64)
}
fn native_server_frame(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    Ok(u64::from(c.context.server_frame as u32))
}
fn cvar_set<const REGISTER: bool, const FORCE: bool>(
    c: &mut Invocation<'_, '_, '_>,
) -> Result<u64, CallError> {
    let name = text_at(c.memory, c.pointer(0)?)?;
    if REGISTER && c.null_pointer(1)? {
        let name = c.services.cvar_name(name, c.context.console.source);
        if let Some(view) = c.services.cvars.bind(name, c.context.console) {
            (ENGINE_CALLS.cvar_reset)(c.services, view, FORCE)?;
        }
        return Ok(0);
    }
    let value = text_at(c.memory, c.pointer(1)?)?;
    write_cvar::<REGISTER, FORCE>(c.services, c.context.console, name, value)
}
fn q3_set_usercmd_value(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    // CL_FinishMove narrows cgame's integer weapon selection to its wire byte.
    let number = c.arg(0)? as u8;
    let weapon = c
        .services
        .registry
        .native_weapon(qa_core::primitives::RuleSetId::Quake3, u16::from(number));
    let scale = float::<false>(c, 1)? as f32;
    (ENGINE_CALLS.set_usercmd_value)(c.services, c.context.console.seat, weapon, scale);
    Ok(0)
}
fn write_cvar<const REGISTER: bool, const FORCE: bool>(
    services: &mut EngineServices<'_>,
    context: qa_console::views::Context,
    name: &str,
    value: &str,
) -> Result<u64, CallError> {
    let name = services.cvar_name(name, context.source);
    let view = match services.cvars.bind(name, context) {
        Some(view) => view,
        None if REGISTER => (ENGINE_CALLS.cvar_register)(services, context, name, Some(value), 0)?,
        None => return Err(CallError::Cvar),
    };
    if REGISTER
        && services
            .cvars
            .read(view)
            .is_ok_and(|text| text.as_str() == value)
    {
        return Ok(0);
    }
    if FORCE {
        // Q3 game/cgame/ui call Cvar_Set -> Cvar_Set2(force=true), while
        // QuakeC keeps its existing normal-write boundary policy.
        (ENGINE_CALLS.cvar_force)(services, view, value)?;
    } else {
        (ENGINE_CALLS.cvar_set)(services, view, value)?;
    }
    Ok(0)
}
fn cvar_set_value(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let value = float::<false>(c, 1)? as f32;
    let mut text = FixedText::<64>::default();
    // Cvar_SetValue uses an integral form only when the f32 fits its native
    // signed int; all other values use %f after promotion to double.
    crate::numbers::native_number(&mut text, f64::from(value), 0, 6)
        .map_err(|_| CallError::Text)?;
    // The original Com_sprintf copies at most 31 bytes into val[32].
    if text.as_bytes().len() >= 32 {
        let _ = (ENGINE_CALLS.console_formatted)(
            c.services,
            format_args!("Com_sprintf: overflow of {} in 32\n", text.as_bytes().len()),
        );
    }
    let name = text_at(c.memory, c.pointer(0)?)?;
    write_cvar::<true, true>(
        c.services,
        c.context.console,
        name,
        &text.as_str()[..text.as_bytes().len().min(31)],
    )
}
fn cvar_reset(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let name = text_at(c.memory, c.pointer(0)?)?;
    let name = c.services.cvar_name(name, c.context.console.source);
    if let Some(view) = c.services.cvars.bind(name, c.context.console) {
        (ENGINE_CALLS.cvar_reset)(c.services, view, false)?;
    }
    Ok(0)
}
fn cvar_create(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let name = text_at(c.memory, c.pointer(0)?)?;
    let value = text_at(c.memory, c.pointer(1)?)?;
    (ENGINE_CALLS.cvar_register)(
        c.services,
        c.context.console,
        name,
        Some(value),
        c.arg(2)? as u32,
    )?;
    Ok(0)
}
// q_shared.h vmCvar_t is identical for QVM and native pointer widths.
const VM_CVAR_BYTES: usize = 16 + 256;
fn q3_cvar_register(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let destination = if c.null_pointer(0)? {
        None
    } else {
        Some(c.pointer(0)?)
    };
    if let Some(address) = destination {
        c.memory.read(address, VM_CVAR_BYTES)?;
    }
    let name = text_at(c.memory, c.pointer(1)?)?;
    let default = text_at(c.memory, c.pointer(2)?)?;
    let view = (ENGINE_CALLS.cvar_register)(
        c.services,
        c.context.console,
        name,
        Some(default),
        c.arg(3)? as u32,
    )?;
    (ENGINE_CALLS.cvar_latch)(c.services, view)?;
    if let Some(address) = destination {
        let handle = (ENGINE_CALLS.cvar_view)(c.services, view)?;
        c.memory.write_word(address, handle as i32)?;
        c.memory.write_word(address + 4, -1)?;
        q3_cvar_project(c, address)?;
    }
    Ok(0)
}
fn q3_cvar_update(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    if c.null_pointer(0)? {
        return Err(CallError::Memory);
    }
    q3_cvar_project(c, c.pointer(0)?)?;
    Ok(0)
}
fn q3_cvar_project(c: &mut Invocation<'_, '_, '_>, address: u64) -> Result<(), CallError> {
    c.memory.read(address, VM_CVAR_BYTES)?;
    let handle = c.memory.read_word(address)? as u32;
    let (view, count) = (ENGINE_CALLS.cvar_snapshot)(c.services, handle)?;
    if c.memory.read_word(address + 4)? == count {
        return Ok(());
    }
    let text = c.services.cvars.read(view).map_err(|_| CallError::Cvar)?;
    let text = text.as_str();
    if text.len() >= 256 {
        return Err(CallError::Text);
    }
    let value = c
        .services
        .cvars
        .numeric(view)
        .map_err(|_| CallError::Cvar)?;
    // The cached integer preserves atoi's projected decimal-prefix semantics.
    let integer = (ENGINE_CALLS.cvar_integer)(c.services, view)?;
    // Q_strncpyz calls strncpy, which pads the native fixed-width field.
    c.memory.read_mut(address + 16, 256)?.fill(0);
    c.memory.write_string(address + 16, 256, text.as_bytes())?;
    c.memory.write_word(address + 8, value.to_bits() as i32)?;
    c.memory.write_word(address + 12, integer)?;
    c.memory.write_word(address + 4, count)?;
    Ok(())
}
fn q2_cvar(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let name_address = c.pointer(0)?;
    let default_address = c.pointer(1)?;
    let flags = c.arg(2)? as u32;
    let name = text_at(c.memory, name_address)?;
    let default = if default_address == 0 {
        None
    } else {
        Some(text_at(c.memory, default_address)?)
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
    let name = text_at(c.memory, name_address)?;
    let value = text_at(c.memory, value_address)?;
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
        let _ = (ENGINE_CALLS.console)(c.services, b"cvar write rejected\n");
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
            let _ = (ENGINE_CALLS.console)(c.services, b"native cvar capacity exceeded\n");
            Ok(0)
        }
        result => result,
    }
}
fn snap_vector(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    if c.null_pointer(0)? {
        return Err(CallError::Memory);
    }
    let address = c.pointer(0)?;
    let vector = qa_core::math::round_vector(c.memory.read_vec3(address)?);
    // Native Sys_SnapVector's FISTP/FILD narrows through signed int32, loses
    // negative zero and returns integer-indefinite for nonfinite/overflow input.
    let vector = qa_core::primitives::Vec3(
        vector
            .0
            .map(|value| crate::numbers::native_integer(value) as f32),
    );
    c.memory.write_vec3(address, vector)?;
    Ok(0)
}
fn cvar_number<const INTEGER: bool>(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let Some(view) = c.services.cvars.bind(c.text(0)?, c.context.console) else {
        return Ok(0);
    };
    Ok(if INTEGER {
        (ENGINE_CALLS.cvar_integer)(c.services, view)? as i64 as u64
    } else {
        (ENGINE_CALLS.cvar_number)(c.services, view)?.to_bits() as u64
    })
}
fn cvar_string(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let target = c.pointer(1)?;
    let length = c.length(2)?;
    let view = c.services.cvars.bind(c.text(0)?, c.context.console);
    if let Some(view) = view {
        let text = c.services.cvars.read(view).map_err(|_| CallError::Cvar)?;
        c.memory
            .write_string(target, length, text.as_str().as_bytes())?;
    } else {
        c.memory.write_string(target, length, b"")?;
    }
    Ok(0)
}
fn argc(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    Ok(c.services
        .client_commands
        .as_deref()
        .map_or_else(|| c.command.len(), |commands| commands.arguments().len()) as u64)
}
fn argv(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let index = c.arg(0)? as u32 as usize;
    let target = c.pointer(1)?;
    let length = c.length(2)?;
    let text = c.services.client_commands.as_deref().map_or_else(
        || c.command.get(index).copied().unwrap_or(b""),
        |commands| commands.arguments().get_bytes(index),
    );
    c.memory.write_string(target, length, text)?;
    Ok(0)
}
fn client_args(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let target = c.pointer(0)?;
    let length = c.length(1)?;
    let mut text = FixedText::<1024>::default();
    c.services
        .client_commands
        .as_deref()
        .ok_or(CallError::Text)?
        .arguments()
        .join_bytes(1, &mut text)
        .map_err(|_| CallError::Text)?;
    c.memory.write_string(target, length, text.as_bytes())?;
    Ok(0)
}
fn client_server_command(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let number = c.arg(0)? as u32 as i32;
    Ok(u64::from((ENGINE_CALLS.server_command)(
        c.services, number,
    )?))
}
fn native_argv(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    Ok(c.native_command
        .ok_or(CallError::Memory)?
        .argv(c.arg(0)? as u32))
}
fn native_args(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    Ok(c.native_command.ok_or(CallError::Memory)?.args())
}
fn command(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let text = text_at(c.memory, c.pointer(1)?)?;
    match c.arg(0)? as u32 {
        1 => (ENGINE_CALLS.command_insert)(c.services, c.context.console, text)?,
        2 => (ENGINE_CALLS.command)(c.services, c.context.console, text)?,
        // EXEC_NOW needs synchronous host dispatch, not queued insertion.
        _ => return Err(CallError::Text),
    }
    Ok(0)
}
fn command_append(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let text = text_at(c.memory, c.pointer(0)?)?;
    (ENGINE_CALLS.command)(c.services, c.context.console, text)?;
    Ok(0)
}
fn file_open(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    if c.arg(2)? != 0 {
        return Err(CallError::File);
    }
    if c.null_pointer(1)? {
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
fn file_seek(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    // QVM and Microsoft long are signed 32-bit; a System V x86-64 long
    // retains the full native word supplied by trap_FS_Seek.
    let offset = match c.addresses {
        Addresses::Native {
            abi: qa_platform::native::NativeAbi::SystemV,
            pointer_bytes: 8,
        }
        | Addresses::NativeFunction {
            abi: qa_platform::native::NativeAbi::SystemV,
            pointer_bytes: 8,
        } => c.arg(1)? as i64,
        _ => i64::from(c.arg(1)? as u32 as i32),
    };
    let from = match c.arg(2)? as u32 {
        0 => std::io::SeekFrom::Current(offset),
        1 => std::io::SeekFrom::End(offset),
        2 if offset >= 0 => std::io::SeekFrom::Start(offset as u64),
        2 => return Ok(u64::from(u32::MAX)),
        _ => return Err(CallError::File),
    };
    Ok(
        if (ENGINE_CALLS.file_seek)(c.services, c.context.module, c.arg(0)? as u32, from)? {
            0
        } else {
            u64::from(u32::MAX)
        },
    )
}
fn entity_token(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let output = c.pointer(0)?;
    let capacity = c.length(1)?;
    if capacity == 0 {
        return Err(CallError::Text);
    }
    c.memory.read(output, capacity)?;
    let token = (ENGINE_CALLS.entity_token)(c.services, c.context.module)?;
    c.memory
        .write_string(output, capacity, token.unwrap_or_default())?;
    Ok(u64::from(token.is_some()))
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
fn native_trace<const WIDE: bool, const CLIP: bool>(
    c: &mut Invocation<'_, '_, '_>,
) -> Result<u64, CallError> {
    use qa_world::collision::{Contents, TraceQuery, trace_policy};
    let output = c.pointer(0)?;
    c.memory.read(output, if WIDE { 96 } else { 72 })?;
    let first = if CLIP { 2 } else { 1 };
    let start = c.vector(first, false)?;
    let end = c.vector(first + 3, false)?;
    let bounds = c.bounds(first + 1, true)?;
    let mut mask = Contents::from_q2(c.arg(6)? as u32);
    if mask.intersects(Contents::SOLID) {
        mask |= Contents::SKY;
    }
    let surfaces = c.native_surfaces.ok_or(CallError::Geometry)?;
    let (rules, entity_rules) = trace_policy(c.context.trace_rules);
    let query = TraceQuery {
        scope: qa_world::collision::TraceScope::Scene,
        start,
        end,
        mins: bounds.mins,
        maxs: bounds.maxs,
        mask,
        pass: qa_core::primitives::CollisionOwner::None,
        rules,
        entity_rules,
        excluded: &[],
    };
    let (result, entity) = if CLIP {
        let address = c.pointer(1)?;
        let entities = c.native_entities.as_mut().ok_or(CallError::Entity)?;
        let target = entities.clip_target(c.services, c.memory, c.context, address)?;
        ((ENGINE_CALLS.clip)(c.services, query, target)?, address)
    } else {
        let entities = c.native_entities.as_ref().ok_or(CallError::Entity)?;
        let view = entities.trace_view(c.memory, c.context)?;
        let query = TraceQuery {
            pass: view.pass(c.pointer(5)?)?,
            ..query
        };
        let result = (ENGINE_CALLS.trace)(c.services, query, Some(&view))?;
        view.finish()?;
        let entity = view.address(&c.services.server.entities, result.entity)?;
        (result, entity)
    };
    crate::traces::write_q2::<WIDE>(c.memory, output, result, surfaces, entity)
}

fn q3_trace(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    use qa_world::collision::{Contents, TraceQuery, trace_policy};
    let output = c.required_pointer(0)?;
    c.memory.read(output, 56)?;
    let start = c.vector(1, false)?;
    let end = c.vector(4, false)?;
    let bounds = c.bounds(2, true)?;
    let mask = Contents::from_q3(c.arg(6)? as u32);
    let (rules, entity_rules) = trace_policy(c.context.trace_rules);
    let view = native_collision_view(c.native_entities.as_deref(), c.memory, c.context)?;
    let number = c.arg(5)? as u32 as i32;
    let pass =
        crate::entities::NativeTraceView::pass_number(view.as_ref(), c.context.module, number)?;
    let query = TraceQuery {
        mins: bounds.mins,
        maxs: bounds.maxs,
        mask,
        pass,
        ..TraceQuery::point(start, end, rules, entity_rules)
    };
    let result = (ENGINE_CALLS.trace)(
        c.services,
        query,
        view.as_ref()
            .map(|view| view as &dyn qa_world::collision::NativeTraceEntities),
    )?;
    if let Some(view) = &view {
        view.finish()?;
    }
    let entity = crate::entities::NativeTraceView::number(
        view.as_ref(),
        &c.services.server.entities,
        result.entity,
    )?;
    crate::traces::write_q3(c.memory, output, result, entity)?;
    Ok(0)
}

fn q3_capsule_unsupported(_: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    Err(CallError::Geometry)
}

fn q3_entity_contact(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    use qa_core::primitives::Vec3;
    use qa_world::collision::{Contents, TraceQuery, trace_policy};
    let bounds = c.bounds(0, false)?;
    let address = c.required_pointer(2)?;
    let projection = c.native_entities.as_mut().ok_or(CallError::Entity)?;
    let target = projection.clip_target(c.services, c.memory, c.context, address)?;
    let (rules, entity_rules) = trace_policy(c.context.trace_rules);
    let query = TraceQuery {
        mins: bounds.mins,
        maxs: bounds.maxs,
        mask: Contents::from_q3(u32::MAX),
        ..TraceQuery::point(Vec3::default(), Vec3::default(), rules, entity_rules)
    };
    Ok(u64::from(
        (ENGINE_CALLS.clip)(c.services, query, target)?.start_solid,
    ))
}

fn native_pmove(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let address = c.pointer(0)?;
    let layout = c.native_pmove.as_ref().ok_or(CallError::Memory)?.layout;
    c.memory.read(address, layout.bytes)?;
    let entities = c.native_entities.as_ref().ok_or(CallError::Entity)?;
    let table = entities.boundary(c.memory, c.context)?;
    let id = if let Some(offset) = layout.player {
        let player = c.memory.read_u64(address + offset as u64)?;
        let id = native_client(c, player)?.ok_or(CallError::Entity)?;
        if c.native_player.is_some_and(|expected| expected != id) {
            return Err(CallError::Entity);
        }
        id
    } else {
        c.native_player.ok_or(CallError::Entity)?
    };
    let client = &mut c.services.server.clients[id.0 as usize];
    let saved = (
        client.player.body,
        client.player.movement,
        client.player.view_angles,
        client.player.view_offset,
        client.command,
    );
    let result = (|| {
        let client = &mut c.services.server.clients[id.0 as usize];
        let codec = c.native_pmove.as_mut().ok_or(CallError::Memory)?;
        codec.read_state(c.memory, address, c.context.module, &mut client.player)?;
        if layout.wide {
            client.player.movement.tuning.n64_physics = c
                .services
                .storage
                .config_integer(c.context.module, crate::q2_abi::RERELEASE_N64_PHYSICS)
                .is_some_and(|value| value != 0);
        }
        client.command = qa_movement::prepare_command(
            client.player.movement_rules,
            codec.command(c.memory, address, client.command.server_time_ms)?,
        );
        let options = codec.options(c.memory, address)?;
        let view = entities.trace_view(c.memory, c.context)?;
        let result = (ENGINE_CALLS.pmove)(c.services, id, Some(&view), options)?;
        view.finish()?;
        let client = &c.services.server.clients[id.0 as usize];
        let player = &client.player;
        let native_address = |id| {
            crate::entities::native_address(
                table,
                c.context.module,
                &c.services.server.entities,
                id,
            )
            .unwrap_or(table.address)
        };
        codec.write_state(c.memory, address, player)?;
        c.memory
            .write_vec3(address + layout.view as u64, player.view_angles)?;
        c.memory
            .write_vec3(address + layout.mins as u64, player.body.mins)?;
        c.memory
            .write_vec3(address + layout.maxs as u64, player.body.maxs)?;
        if let Some(offset) = layout.height {
            c.memory.write_word(
                address + offset as u64,
                player.view_offset.0[2].to_bits() as i32,
            )?;
        }
        if let Some(outputs) = &layout.outputs {
            c.memory.write(
                address + outputs.jump_sound as u64,
                &[u8::from(result.jump_sound)],
            )?;
            c.memory.write(
                address + outputs.step_clip as u64,
                &[u8::from(result.step_clip)],
            )?;
            c.memory.write_word(
                address + outputs.impact_delta as u64,
                result.impact_delta.to_bits() as i32,
            )?;
        }
        c.memory.write_u64(
            address + layout.ground as u64,
            if player.movement.grounded
                && !(!layout.wide
                    && player.movement.mode == qa_core::primitives::MovementMode::Spectator)
            {
                player
                    .movement
                    .ground
                    .map(native_address)
                    .unwrap_or(table.address)
            } else {
                0
            },
        )?;
        c.memory.write_word(
            address + layout.water as u64,
            qa_world::collision::Contents(player.movement.water_contents).to_q2() as i32,
        )?;
        if layout.wide {
            c.memory.write(
                address + layout.level as u64,
                &[player.movement.water_level],
            )?;
        } else {
            c.memory.write_word(
                address + layout.level as u64,
                i32::from(player.movement.water_level),
            )?;
        }
        c.memory.write_word(
            address + layout.touch_count as u64,
            result.contact_count as i32,
        )?;
        for (index, trace) in result.contacts[..result.contact_count]
            .iter()
            .flatten()
            .enumerate()
        {
            let entity = trace.entity.map(native_address).unwrap_or(table.address);
            let at = address + layout.touches as u64;
            if layout.wide {
                crate::traces::write_q2::<true>(
                    c.memory,
                    at + (index * 96) as u64,
                    *trace,
                    c.native_surfaces.ok_or(CallError::Geometry)?,
                    entity,
                )?;
            } else {
                c.memory.write_u64(at + (index * 8) as u64, entity)?;
            }
        }
        Ok(0)
    })();
    if result.is_err() {
        let client = &mut c.services.server.clients[id.0 as usize];
        (
            client.player.body,
            client.player.movement,
            client.player.view_angles,
            client.player.view_offset,
            client.command,
        ) = saved;
    }
    result
}
fn native_collision_view<'a, 'memory>(
    projection: Option<&'a crate::entities::EntityProjection>,
    memory: &'a crate::memory::ModuleMemory<'memory>,
    context: CallContext,
) -> Result<Option<crate::entities::NativeTraceView<'a, 'memory>>, CallError> {
    projection
        .map(|projection| projection.optional_trace_view(memory, context))
        .transpose()
        .map(Option::flatten)
}

fn native_point_contents(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let point = c.memory.read_vec3(c.pointer(0)?)?;
    if !point.is_finite() {
        return Err(CallError::Geometry);
    }
    let (_, rules) = qa_world::collision::trace_policy(c.context.trace_rules);
    let view = native_collision_view(c.native_entities.as_deref(), c.memory, c.context)?;
    let contents = (ENGINE_CALLS.point_contents)(
        c.services,
        point,
        rules,
        qa_core::primitives::CollisionOwner::None,
        view.as_ref()
            .map(|view| view as &dyn qa_world::collision::NativeTraceEntities),
    )?;
    if let Some(view) = view {
        view.finish()?;
    }
    Ok(u64::from(contents.to_q2()))
}

fn qc_point_contents(
    c: &mut QuakeCCalls<'_, '_>,
    vm: &mut quakec::Vm,
    arguments: &[u64],
) -> Result<CallValue, CallError> {
    let point = qc_vector(arguments, 0)?;
    let (_, rules) = qa_world::collision::trace_policy(c.context.trace_rules);
    let view = if rules
        .filtering
        .contains(qa_world::collision::EntityTraceFlags::LINKED_CONTENTS)
    {
        Some(
            vm.projection
                .quakec_trace_view(&vm.entities, &vm.strings, c.context, 0)?
                .0,
        )
    } else {
        None
    };
    let contents = (ENGINE_CALLS.point_contents)(
        c.services,
        point,
        rules,
        qa_core::primitives::CollisionOwner::None,
        view.as_ref()
            .map(|view| view as &dyn qa_world::collision::NativeTraceEntities),
    )?;
    if let Some(view) = view {
        view.finish()?;
    }
    Ok(CallValue::Word(bits::<false>(f64::from(contents.to_q1()))))
}

fn q3_usercmd(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let slot = c.arg(0)? as i32;
    let limit = c
        .services
        .storage
        .max_clients(c.services.cvars, c.context.native_rules);
    if slot < 0 || slot >= limit {
        return Err(CallError::Entity);
    }
    // Valid unused native client rows contain a zero lastUsercmd in qsrc.
    let command = (ENGINE_CALLS.usercmd)(
        c.services,
        qa_core::primitives::NativeEntity {
            module: c.context.module,
            slot,
        },
    )
    .unwrap_or_default();
    let weapon = c.services.registry.command_weapon_number(command.weapon);
    let native = qa_network::commands::to_q3_usercmd(&command, weapon);
    let target = c.pointer(1)?;
    let output = c.memory.read_mut(target, 24)?;
    for (bytes, word) in output[..20].chunks_exact_mut(4).zip([
        native.server_time as u32,
        native.angles[0] as u32,
        native.angles[1] as u32,
        native.angles[2] as u32,
        native.buttons,
    ]) {
        bytes.copy_from_slice(&word.to_le_bytes());
    }
    output[20] = native.weapon;
    for (byte, movement) in output[21..].iter_mut().zip(native.movement) {
        *byte = movement as u8;
    }
    Ok(0)
}

fn q3_point_contents(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let point = c.memory.read_vec3(c.pointer(0)?)?;
    let pass = qa_core::primitives::CollisionOwner::Native(qa_core::primitives::NativeEntity {
        module: c.context.module,
        slot: c.arg(1)? as i32,
    });
    let rules = qa_world::collision::trace_policy(c.context.trace_rules).1;
    let view = native_collision_view(c.native_entities.as_deref(), c.memory, c.context)?;
    let contents = (ENGINE_CALLS.point_contents)(
        c.services,
        point,
        rules,
        pass,
        view.as_ref()
            .map(|view| view as &dyn qa_world::collision::NativeTraceEntities),
    )?;
    if let Some(view) = view {
        view.finish()?;
    }
    Ok(u64::from(contents.to_q3()))
}

fn native_info_value(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let source = c.pointer(0)?;
    let span =
        qa_core::text::info_value(c.memory.cstring(source)?, c.string(1)?, false).unwrap_or(0..0);
    let capacity = c.length(3)?;
    if capacity != 0 {
        let output = c.pointer(2)?;
        let length = span.len().min(capacity - 1);
        c.memory.read(output, length + 1)?;
        c.memory.copy(output, source + span.start as u64, length)?;
        c.memory.write(output + length as u64, &[0])?;
    }
    Ok(span.len() as u64)
}

fn native_info_remove(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let source = c.pointer(0)?;
    let mut key = FixedText::<512>::default();
    if key.set_bytes(c.string(1)?).is_err() {
        return Ok(0);
    }
    let (_, removed) = remove_native_info(c.memory, source, key.as_bytes())?;
    Ok(u64::from(removed))
}

fn remove_native_info(
    memory: &mut ModuleMemory<'_>,
    source: u64,
    key: &[u8],
) -> Result<(usize, bool), CallError> {
    let len = memory.cstring(source)?.len();
    let info = memory.read_mut(source, len + 1)?;
    let remaining = qa_core::text::info_remove(&mut info[..len], key, false, true);
    info[remaining] = 0;
    Ok((remaining, remaining != len))
}

fn native_info_set(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    // q2repro Info_SubValidate checks raw lengths and masked delimiters before
    // removing a key. Append then strips high bits and non-printable bytes.
    use crate::q2_abi::{INFO_KEY_BYTES, INFO_VALUE_BYTES, client_layout};
    use qa_core::text::{InfoBytes, InfoRules, InfoUpdate};
    let mut key = FixedText::<INFO_KEY_BYTES>::default();
    let mut value = FixedText::<INFO_VALUE_BYTES>::default();
    if key.set_bytes(c.string(1)?).is_err() || value.set_bytes(c.string(2)?).is_err() {
        return Ok(0);
    }
    let layout = client_layout(c.context.native_rules).ok_or(CallError::Text)?;
    let Ok(update) = InfoUpdate::new(
        key.as_bytes(),
        value.as_bytes(),
        InfoRules {
            bytes: layout.userinfo_bytes,
            key_bytes: INFO_KEY_BYTES,
            value_bytes: layout.value_bytes,
            filter: InfoBytes::Ascii7Printable,
        },
    ) else {
        return Ok(0);
    };
    let source = c.pointer(0)?;
    let mut length = c.memory.cstring(source)?.len();
    let output = c.memory.read_mut(source, layout.userinfo_bytes)?;
    Ok(u64::from(update.apply(output, &mut length).is_ok()))
}

fn native_client(
    c: &Invocation<'_, '_, '_>,
    address: u64,
) -> Result<Option<qa_core::primitives::ClientId>, CallError> {
    let entities = c.native_entities.as_ref().ok_or(CallError::Entity)?;
    let native = entities.identity(c.memory, c.context, address)?;
    Ok(c.services.server.client_for_native(native))
}
fn native_delivery<const WIDE: bool>(
    c: &Invocation<'_, '_, '_>,
    target: qa_core::primitives::NativeTarget,
    origin: qa_core::primitives::Vec3,
    reliable: bool,
    dupe_key: u32,
) -> qa_core::primitives::NativeDelivery {
    let source = c.context.native_rules;
    qa_core::primitives::NativeDelivery {
        module: c.context.module,
        source,
        target,
        origin,
        reliable,
        dupe_key,
        component: 0,
        emission: c.services.storage.emission(c.context),
        check_portals: !c.services.ignore_areas(source, WIDE),
    }
}
fn native_multicast<const WIDE: bool>(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    use qa_core::primitives::{NativeTarget, Vec3};
    let to = c.arg(1)? as u32 as i32;
    if !(0..=if WIDE { 2 } else { 5 }).contains(&to) {
        return Err(CallError::Memory);
    }
    let (to, reliable) = if WIDE {
        (to, c.arg(2)? as u8 != 0)
    } else {
        (to % 3, to >= 3)
    };
    let target = match to {
        0 => NativeTarget::All,
        1 => NativeTarget::Hearing,
        2 => NativeTarget::Sight,
        _ => return Err(CallError::Memory),
    };
    // ALL ignores origin, as in SV_Multicast; PVS/PHS require a valid vector.
    let origin = if target == NativeTarget::All {
        Vec3::default()
    } else {
        c.memory.read_vec3(c.pointer(0)?)?
    };
    let delivery = native_delivery::<WIDE>(c, target, origin, reliable, 0);
    (ENGINE_CALLS.native_message)(c.services, delivery, c.context.sound_policy)?;
    Ok(0)
}
fn native_unicast<const WIDE: bool>(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let address = c.pointer(0)?;
    let client = if address == 0 {
        None
    } else {
        native_client(c, address)?
    };
    let Some(client) = client else {
        if WIDE {
            c.services.storage.reset_message(c.context.module)?;
        }
        return Ok(0);
    };
    let reliable = if WIDE {
        c.arg(1)? as u8 != 0
    } else {
        c.arg(1)? as u32 != 0
    };
    let dupe_key = if WIDE { c.arg(2)? as u32 } else { 0 };
    let delivery = native_delivery::<WIDE>(
        c,
        qa_core::primitives::NativeTarget::Client(client),
        qa_core::primitives::Vec3::default(),
        reliable,
        dupe_key,
    );
    (ENGINE_CALLS.native_message)(c.services, delivery, c.context.sound_policy)?;
    Ok(0)
}

fn native_message_bits<const WIDTH: u8>(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    (ENGINE_CALLS.message_bits)(c.services, c.context.module, c.arg(0)? as u32, WIDTH)?;
    Ok(0)
}
fn native_message_position<const FLOAT: bool>(
    c: &mut Invocation<'_, '_, '_>,
) -> Result<u64, CallError> {
    let position = c.memory.read_vec3(c.pointer(0)?)?;
    for value in position.0 {
        let (word, width) = if FLOAT {
            (value.to_bits(), 32)
        } else {
            (qa_core::math::eighth_to_short(value) as i32 as u32, 16)
        };
        (ENGINE_CALLS.message_bits)(c.services, c.context.module, word, width)?;
    }
    Ok(0)
}
fn native_message_direction(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let pointer = c.pointer(0)?;
    let direction = if pointer == 0 {
        qa_core::primitives::Vec3::default()
    } else {
        c.memory.read_vec3(pointer)?
    };
    (ENGINE_CALLS.message_bits)(
        c.services,
        c.context.module,
        u32::from(qa_core::math::direction_to_byte(direction)),
        8,
    )?;
    Ok(0)
}
fn native_message_angle(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let word = qa_core::math::angle_to_byte(
        float::<false>(c, 0)? as f32,
        qa_core::math::AngleByteForm::Float,
    );
    (ENGINE_CALLS.message_bits)(c.services, c.context.module, word as u32, 8)?;
    Ok(0)
}
fn native_message_entity(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let entities = c.native_entities.as_ref().ok_or(CallError::Entity)?;
    let native = entities.identity(c.memory, c.context, c.pointer(0)?)?;
    (ENGINE_CALLS.message_bits)(c.services, c.context.module, native.slot as u32, 16)?;
    Ok(0)
}
fn native_visibility<const WIDE: bool, const HEARING: bool>(
    c: &mut Invocation<'_, '_, '_>,
) -> Result<u64, CallError> {
    let channel = if HEARING {
        qa_world::visibility::VisibilityChannel::Hearing
    } else {
        qa_world::visibility::VisibilityChannel::Sight
    };
    let portals =
        (!WIDE || c.arg(2)? != 0) && !c.services.ignore_areas(c.context.native_rules, WIDE);
    visibility_import(
        c,
        qa_world::visibility::PointVisibility {
            channel,
            missing_visible: false,
            area_zero: !WIDE,
        },
        portals,
    )
}
fn q3_visibility<const IGNORE_PORTALS: bool>(
    c: &mut Invocation<'_, '_, '_>,
) -> Result<u64, CallError> {
    let portals = !IGNORE_PORTALS && !c.services.ignore_areas(c.context.native_rules, true);
    visibility_import(
        c,
        qa_world::visibility::PointVisibility {
            channel: qa_world::visibility::VisibilityChannel::Sight,
            missing_visible: true,
            area_zero: true,
        },
        portals,
    )
}
fn visibility_import(
    c: &mut Invocation<'_, '_, '_>,
    policy: qa_world::visibility::PointVisibility,
    portals: bool,
) -> Result<u64, CallError> {
    let first = c.memory.read_vec3(c.pointer(0)?)?;
    let second = c.memory.read_vec3(c.pointer(1)?)?;
    Ok(u64::from((ENGINE_CALLS.in_visibility)(
        c.services, first, second, policy, portals,
    )?))
}
fn native_message_string(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let pointer = c.pointer(0)?;
    let bytes = if pointer == 0 {
        &[][..]
    } else {
        c.memory.cstring(pointer)?
    };
    (ENGINE_CALLS.message_string)(c.services, c.context.module, bytes)?;
    Ok(0)
}

fn native_portal_set<const WIDE: bool>(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let number = c.arg(0)? as u32 as i32;
    if number < 0 && WIDE {
        return Ok(0);
    }
    let number = u32::try_from(number).map_err(|_| CallError::Geometry)?;
    (ENGINE_CALLS.portal_set)(c.services, number, c.arg(1)? != 0, !WIDE)?;
    Ok(0)
}

fn native_areas_connected<const WIDE: bool>(
    c: &mut Invocation<'_, '_, '_>,
) -> Result<u64, CallError> {
    if c.services.world.is_none() {
        return Ok(0);
    }
    area_connection(c, c.context.native_rules, WIDE, i32::from(WIDE), false)
}

fn q3_areas_connected(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    area_connection(c, c.context.native_rules, true, 0, true)
}

fn area_connection(
    c: &mut Invocation<'_, '_, '_>,
    source: qa_core::primitives::RuleSetId,
    integer: bool,
    minimum: i32,
    strict: bool,
) -> Result<u64, CallError> {
    if c.services.ignore_areas(source, integer) {
        return Ok(1);
    }
    let first = c.arg(0)? as u32 as i32;
    let second = c.arg(1)? as u32 as i32;
    if first < minimum || second < minimum {
        return Ok(0);
    }
    let connected = (ENGINE_CALLS.areas_connected)(c.services, first as u32, second as u32);
    Ok(u64::from(if strict {
        connected.ok_or(CallError::Geometry)?
    } else {
        connected.unwrap_or(false)
    }))
}

fn native_area_query<const WIDE: bool>(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    use qa_world::area::LinkFlags;
    let bounds = c.bounds(0, false)?;
    let role = match c.arg(4)? as u32 as i32 {
        1 => LinkFlags::SOLID,
        2 => LinkFlags::TRIGGER,
        _ => return Err(CallError::Geometry),
    };
    let max = if WIDE {
        c.length(3)?
    } else {
        usize::try_from(c.arg(3)? as u32 as i32).map_err(|_| CallError::Memory)?
    };
    // The owned child requests unfiltered candidates, then applies the
    // callback after the engine's shared-memory borrows have ended.
    if WIDE && c.pointer(5)? != 0 {
        return Err(CallError::Aborted);
    }
    if !WIDE && max == 0 {
        return Ok(0);
    }
    let output = c.pointer(2)?;
    area_query(
        c,
        bounds,
        role,
        max,
        (max != 0 && output != 0).then_some(output),
        false,
    )
}

fn q3_entities_in_box(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let bounds = c.bounds(0, false)?;
    let max = usize::try_from(c.arg(3)? as u32 as i32).map_err(|_| CallError::Memory)?;
    if max == 0 {
        return Ok(0);
    }
    let output = c.required_pointer(2)?;
    area_query(
        c,
        bounds,
        qa_world::area::LinkFlags::LINKED,
        max,
        Some(output),
        true,
    )
}

fn area_query(
    c: &mut Invocation<'_, '_, '_>,
    bounds: qa_core::primitives::Bounds,
    role: qa_world::area::LinkFlags,
    max: usize,
    output: Option<u64>,
    numbers: bool,
) -> Result<u64, CallError> {
    let projection = c.native_entities.as_ref().ok_or(CallError::Entity)?;
    if numbers && !projection.ready() {
        if let Some(output) = output {
            c.memory.read(output, 4)?;
        }
        return Ok(0);
    }
    let native = projection.boundary(c.memory, c.context)?;
    let limit = if max == 0 { usize::MAX } else { max };
    let limit = limit
        .min(native.capacity as usize)
        .min(c.services.server.entities.capacity());
    let width = if numbers { 4 } else { 8 };
    let list = match output {
        Some(output) => c.memory.read_mut(output, limit * width)?,
        None => &mut [],
    };
    let mut count = 0;
    (ENGINE_CALLS.area_query)(c.services, bounds, role, &mut |id| {
        let Some(slot) =
            crate::entities::native_slot(native, c.context.module, &c.services.server.entities, id)
        else {
            return true;
        };
        let value = if numbers {
            u64::from(slot)
        } else {
            let Some(address) = crate::entities::slot_address(native, slot) else {
                return true;
            };
            address
        };
        if !list.is_empty() {
            list[count * width..count * width + width]
                .copy_from_slice(&value.to_le_bytes()[..width]);
        }
        count += 1;
        count < limit
    });
    Ok(count as u64)
}

fn q3_locate_game_data(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    if c.null_pointer(0)? || c.null_pointer(3)? {
        return Err(CallError::Memory);
    }
    let count = u32::try_from(c.arg(1)? as u32 as i32).map_err(|_| CallError::Entity)?;
    let stride = u64::try_from(c.arg(2)? as u32 as i32).map_err(|_| CallError::Entity)?;
    let player_stride = u64::try_from(c.arg(4)? as u32 as i32).map_err(|_| CallError::Entity)?;
    let address = c.pointer(0)?;
    let players = c.pointer(3)?;
    let player_count = u32::try_from(
        c.services
            .storage
            .max_clients(c.services.cvars, c.context.native_rules),
    )
    .map_err(|_| CallError::Entity)?;
    let entities = crate::entities::Entities {
        address,
        stride,
        count,
        capacity: count,
        server_flags: 0,
    };
    c.native_entities
        .as_mut()
        .ok_or(CallError::Entity)?
        .register_game_data(c.memory, entities, players, player_stride, player_count)?;
    Ok(0)
}

fn native_set_model(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let address = c.pointer(0)?;
    let name = c.pointer(1)?;
    let entities = c.native_entities.as_mut().ok_or(CallError::Entity)?;
    entities.set_model(c.services, c.memory, c.context, address, name)?;
    Ok(0)
}
fn q3_set_brush_model(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let address = c.required_pointer(0)?;
    let name = c.required_pointer(1)?;
    c.native_entities
        .as_mut()
        .ok_or(CallError::Entity)?
        .set_model(c.services, c.memory, c.context, address, name)?;
    Ok(0)
}
fn native_link(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    if c.null_pointer(0)? {
        return Err(CallError::Entity);
    }
    let address = c.pointer(0)?;
    let entities = c.native_entities.as_mut().ok_or(CallError::Entity)?;
    entities.link(c.services, c.memory, c.context, address)?;
    Ok(0)
}
fn native_unlink(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    if c.null_pointer(0)? {
        return Err(CallError::Entity);
    }
    let address = c.pointer(0)?;
    let entities = c.native_entities.as_mut().ok_or(CallError::Entity)?;
    entities.unlink(c.services, c.memory, c.context, address)?;
    Ok(0)
}
fn native_forget_observer(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let address = c.pointer(0)?;
    let entities = c.native_entities.as_mut().ok_or(CallError::Entity)?;
    entities.forget_observer(c.services, c.memory, c.context, address)?;
    Ok(0)
}
fn native_register_observer(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let address = c.pointer(0)?;
    let entities = c.native_entities.as_mut().ok_or(CallError::Entity)?;
    entities.register_observer(c.services, c.memory, c.context, address)?;
    Ok(0)
}
fn config_set(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let pointer = c.pointer(1)?;
    let ordinal = c.length(0)?;
    let text = if pointer == 0 {
        &[][..]
    } else {
        c.memory.cstring(pointer)?
    };
    let text = if let Some(native) = &c.native_configs {
        &text[..text.len().min(native.capacity(ordinal)? - 1)]
    } else {
        text
    };
    (ENGINE_CALLS.configstring)(c.services, c.context.module, ordinal, text)?;
    Ok(0)
}
fn native_config_get(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let ordinal = c.length(0)?;
    c.native_configs
        .as_mut()
        .ok_or(CallError::ConfigString)?
        .publish(c.services.storage, c.context.module, c.memory, ordinal)
}
fn config_get(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let target = c.pointer(1)?;
    let length = c.length(2)?;
    let (text, _) = c
        .services
        .storage
        .configstring(c.context.module, c.length(0)?)?;
    c.memory.write_string(target, length, text)?;
    Ok(0)
}
fn memset(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let target = c.pointer(0)?;
    let value = c.arg(1)? as u8;
    let length = c.length(2)?;
    c.memory.read_mut(target, length)?.fill(value);
    Ok(if matches!(c.addresses, Addresses::NativeFunction { .. }) {
        target
    } else {
        0
    })
}
fn memcpy(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    c.memory.copy(c.pointer(0)?, c.pointer(1)?, c.length(2)?)?;
    Ok(if matches!(c.addresses, Addresses::NativeFunction { .. }) {
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
fn rint(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let value = float::<false>(c, 0)?;
    // PF_rint promotes the parameter before adding the double literal. Do not
    // round the biased value to float before the native integer conversion.
    let biased = value + if value > 0.0 { 0.5 } else { -0.5 };
    Ok(bits::<false>(f64::from(crate::numbers::native_integer(
        biased,
    ))))
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
