//! Windows kernel services: modules, heaps, memory, TLS, files, locale.
//!
//! Donor: `src/guest/runtime/windows/kernel.ts`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::core::contracts::{
    GuestAddress, GuestAllocationOptions, GuestCallResult, GuestCallValue, GuestMapOptions,
    GuestPermissions, GuestStorage,
};
use crate::core::memory::SparseGuestMemory;
use crate::error::GuestError;
use crate::runtime::common::memory::{
    allocate_native_memory, count, integer, pointer, read_string, read_unsigned, required_pointer,
    string_bytes, write_pointer, write_unsigned,
};
use crate::runtime::windows::contracts::{
    invoke_nested, now_millis, set_last_error, unsupported_windows, SharedWindows, WindowsContext,
    WindowsFile, WindowsOpenOptions, WindowsServiceRegistrar, WindowsStream,
};
use crate::runtime::windows::time::{local_offset_at, system_time_fields};

fn u32_result(value: u32) -> GuestCallResult {
    GuestCallResult::Value(GuestCallValue::Uint32(value))
}

fn bool_result(value: bool) -> GuestCallResult {
    GuestCallResult::Value(GuestCallValue::Int32(i32::from(value)))
}

fn ptr_result(value: Option<GuestAddress>) -> GuestCallResult {
    GuestCallResult::Value(GuestCallValue::Pointer(value))
}

fn void_result() -> GuestCallResult {
    GuestCallResult::Void
}

struct VirtualReservation {
    address: GuestAddress,
    size: usize,
}

struct FlsSlot {
    callback: Option<GuestAddress>,
    value: Option<GuestAddress>,
}

/// Open handle target.
enum FileKind {
    /// Standard input.
    Stdin,
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
    /// Host file.
    File(Box<dyn WindowsFile>),
}

struct FileHandle {
    file: FileKind,
    offset: usize,
}

/// Mutable kernel state shared with host closures.
#[derive(Default)]
pub struct KernelState {
    heaps: HashSet<u64>,
    reservations: HashMap<u64, VirtualReservation>,
    tls_slots: HashSet<u32>,
    fls: HashMap<u32, FlsSlot>,
    locks: HashSet<u64>,
    exception_filter: Option<GuestAddress>,
    handles: HashMap<u64, FileHandle>,
    standards: HashMap<i32, GuestAddress>,
}

/// Shared handle to [`KernelState`].
pub type KernelShared = Rc<RefCell<KernelState>>;

fn stored_string(
    memory: &mut SparseGuestMemory,
    value: &str,
    wide: bool,
) -> Result<GuestAddress, GuestError> {
    let bytes = string_bytes(value, wide);
    let address = allocate_native_memory(memory, bytes.len(), "Windows process string")?;
    memory.write(address, &bytes)?;
    Ok(address)
}

fn pointer_secret(width: usize) -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0x1234_5678_9abc_def0);
    let mut x = nanos ^ (std::process::id() as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    (if width == 4 { x as u32 as u64 } else { x }) | 1
}

fn performance_micros() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_micros() as u64
}

/// Register the kernel32 service set.
pub fn install_kernel(host: &mut WindowsServiceRegistrar<'_>) -> Result<(), GuestError> {
    let width = host.pointer_bytes;
    let teb = host.teb;
    let process_id = host.process_id;
    let thread_id = host.thread_id;
    let shared = Rc::clone(&host.shared);
    let context = host.context.clone();
    let command_line = shared
        .borrow()
        .capabilities
        .command_line
        .clone()
        .unwrap_or_else(|| "\"quake-typescript.exe\"".to_string());
    let command_line_a = stored_string(host.memory, &command_line, false)?;
    let command_line_w = stored_string(host.memory, &command_line, true)?;
    let mut environment: Vec<(String, String)> = shared
        .borrow()
        .capabilities
        .environment
        .clone()
        .map(|map| map.into_iter().collect())
        .unwrap_or_default();
    environment.sort_by(|left, right| left.0.cmp(&right.0));
    let environment_text = environment
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\0")
        + "\0";
    let environment_a = stored_string(host.memory, &environment_text, false)?;
    let environment_w = stored_string(host.memory, &environment_text, true)?;
    let invalid = host.memory.pointer(if width == 8 { u64::MAX } else { u32::MAX as u64 })?;
    let pointer_secret = pointer_secret(width);

    for name in ["EncodePointer", "DecodePointer"] {
        host.service("kernel32.dll", name, &[GuestStorage::Pointer], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let memory = ctx.memory();
                let raw = pointer(args, 0)?.map(|address| address.offset).unwrap_or(0);
                Ok(ptr_result(memory.pointer(raw ^ pointer_secret)?))
            },
        ))?;
    }

    let process_heap = host.memory.allocate(&GuestAllocationOptions {
        byte_length: 16,
        alignment: 16,
        permissions: GuestPermissions::ReadWrite,
        label: "Windows process heap".to_string(),
    })?;
    let kernel: KernelShared = Rc::new(RefCell::new(KernelState {
        heaps: [process_heap.offset].into_iter().collect(),
        ..Default::default()
    }));

    {
        host.service("kernel32.dll", "GetProcessHeap", &[], Some(GuestStorage::Pointer), Rc::new(
            move |_, _, _| Ok(ptr_result(Some(process_heap))),
        ))?;
    }
    {
        let width_copy = width;
        let teb_copy = teb;
        host.service("kernel32.dll", "GetLastError", &[], Some(GuestStorage::Uint32), Rc::new(
            move |ctx, _, _| {
                let offset = if width_copy == 4 { 0x34 } else { 0x68 };
                let memory = ctx.memory();
                Ok(u32_result(memory.read_u32(memory.offset(teb_copy, offset)?)?))
            },
        ))?;
    }
    host.service("kernel32.dll", "SetLastError", &[GuestStorage::Uint32], None, Rc::new(
        move |ctx, _, args| {
            set_last_error(ctx.memory(), teb, integer(args, 0)? as u32)?;
            Ok(void_result())
        },
    ))?;
    host.service("kernel32.dll", "GetCurrentThreadId", &[], Some(GuestStorage::Uint32), Rc::new(
        move |_, _, _| Ok(u32_result(thread_id)),
    ))?;
    host.service("kernel32.dll", "GetCurrentProcessId", &[], Some(GuestStorage::Uint32), Rc::new(
        move |_, _, _| Ok(u32_result(process_id)),
    ))?;
    host.service("kernel32.dll", "GetCurrentProcess", &[], Some(GuestStorage::Pointer), Rc::new(
        move |_, _, _| Ok(ptr_result(invalid)),
    ))?;
    host.service("kernel32.dll", "GetVersion", &[], Some(GuestStorage::Uint32), Rc::new(
        move |_, _, _| Ok(u32_result(0x0565_0004)),
    ))?;
    host.service("kernel32.dll", "GetCommandLineA", &[], Some(GuestStorage::Pointer), Rc::new(
        move |_, _, _| Ok(ptr_result(Some(command_line_a))),
    ))?;
    host.service("kernel32.dll", "GetCommandLineW", &[], Some(GuestStorage::Pointer), Rc::new(
        move |_, _, _| Ok(ptr_result(Some(command_line_w))),
    ))?;
    host.service("kernel32.dll", "GetACP", &[], Some(GuestStorage::Uint32), Rc::new(
        move |_, _, _| Ok(u32_result(1252)),
    ))?;
    host.service("kernel32.dll", "GetOEMCP", &[], Some(GuestStorage::Uint32), Rc::new(
        move |_, _, _| Ok(u32_result(437)),
    ))?;
    host.service("kernel32.dll", "IsValidCodePage", &[GuestStorage::Uint32], Some(GuestStorage::Int32), Rc::new(
        move |_, _, args| {
            Ok(bool_result([1252, 437, 65001].contains(&(integer(args, 0)? as i32))))
        },
    ))?;
    host.service("kernel32.dll", "GetCPInfo", &[GuestStorage::Uint32, GuestStorage::Pointer], Some(GuestStorage::Int32), Rc::new(
        move |ctx, _, args| {
            let codepage = integer(args, 0)? as i32;
            if ![0, 1, 1252, 437, 65001].contains(&codepage) {
                set_last_error(ctx.memory(), teb, 87)?;
                return Ok(bool_result(false));
            }
            let memory = ctx.memory();
            let address = required_pointer(args, 1)?;
            memory.write(address, &[0u8; 20])?;
            write_unsigned(memory, address, 4, if codepage == 65001 { 4 } else { 1 })?;
            write_unsigned(memory, memory.offset(address, 4)?, 2, 63)?;
            Ok(bool_result(true))
        },
    ))?;
    for (name, value) in [
        ("GetEnvironmentStrings", environment_a),
        ("GetEnvironmentStringsA", environment_a),
        ("GetEnvironmentStringsW", environment_w),
    ] {
        host.service("kernel32.dll", name, &[], Some(GuestStorage::Pointer), Rc::new(
            move |_, _, _| Ok(ptr_result(Some(value))),
        ))?;
    }
    for name in ["FreeEnvironmentStringsA", "FreeEnvironmentStringsW"] {
        host.service("kernel32.dll", name, &[GuestStorage::Pointer], Some(GuestStorage::Int32), Rc::new(
            move |_, _, _| Ok(bool_result(true)),
        ))?;
    }
    for wide in [false, true] {
        let suffix = if wide { "W" } else { "A" };
        {
            let shared = Rc::clone(&shared);
            let context = context.clone();
            host.service("kernel32.dll", &format!("GetModuleHandle{suffix}"), &[GuestStorage::Pointer], Some(GuestStorage::Pointer), Rc::new(
                move |ctx, _, args| {
                    let memory = ctx.memory();
                    let name = pointer(args, 0)?;
                    let handle = match name {
                        None => shared.borrow().images.first().map(|image| image.image.base),
                        Some(name) => {
                            let text = read_string(memory, name, wide)?;
                            context.library_handle(&text)
                        }
                    };
                    if handle.is_none() {
                        set_last_error(ctx.memory(), teb, 126)?;
                    }
                    Ok(ptr_result(handle))
                },
            ))?;
        }
        {
            let context = context.clone();
            host.service("kernel32.dll", &format!("LoadLibrary{suffix}"), &[GuestStorage::Pointer], Some(GuestStorage::Pointer), Rc::new(
                move |ctx, _, args| {
                    let memory = ctx.memory();
                    let name = read_string(memory, required_pointer(args, 0)?, wide)?;
                    let handle = context.load_library(memory, teb, &name)?;
                    Ok(ptr_result(handle))
                },
            ))?;
        }
        {
            let context = context.clone();
            host.service("kernel32.dll", &format!("LoadLibraryEx{suffix}"), &[GuestStorage::Pointer, GuestStorage::Pointer, GuestStorage::Uint32], Some(GuestStorage::Pointer), Rc::new(
                move |ctx, _, args| {
                    let memory = ctx.memory();
                    let name = pointer(args, 0)?;
                    let reserved = pointer(args, 1)?;
                    let flags = integer(args, 2)? as u32;
                    if name.is_none() || reserved.is_some() || (flags & !0x3fff) != 0 {
                        set_last_error(memory, teb, 87)?;
                        return Ok(ptr_result(None));
                    }
                    if (flags & !0x1f00) != 0 {
                        set_last_error(memory, teb, 50)?;
                        return Ok(ptr_result(None));
                    }
                    let library = read_string(memory, name.expect("name checked"), wide)?;
                    if library.is_empty() || (flags & 0x100) != 0 && !is_absolute_path(&library) {
                        set_last_error(memory, teb, 87)?;
                        return Ok(ptr_result(None));
                    }
                    let handle = context.load_library(memory, teb, &library)?;
                    Ok(ptr_result(handle))
                },
            ))?;
        }
    }
    {
        let context = context.clone();
        host.service("kernel32.dll", "FreeLibrary", &[GuestStorage::Pointer], Some(GuestStorage::Int32), Rc::new(
            move |ctx, _, args| {
                let handle = pointer(args, 0)?;
                let Some(handle) = handle else {
                    set_last_error(ctx.memory(), teb, 6)?;
                    return Ok(bool_result(false));
                };
                let memory = ctx.memory();
                Ok(bool_result(context.free_library(memory, teb, handle)?))
            },
        ))?;
    }
    {
        let context = context.clone();
        host.service("kernel32.dll", "GetProcAddress", &[GuestStorage::Pointer, GuestStorage::Pointer], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let memory = ctx.memory();
                let handle = pointer(args, 0)?;
                let name = pointer(args, 1)?;
                let (Some(handle), Some(name)) = (handle, name) else {
                    set_last_error(memory, teb, 127)?;
                    return Ok(ptr_result(None));
                };
                let library = context.library_name(handle);
                let address = match library {
                    None => None,
                    Some(library) => {
                        let symbol = if name.offset <= 65535 {
                            format!("#{}", name.offset)
                        } else {
                            read_string(memory, name, false)?
                        };
                        context.resolve_address(&library, &symbol)
                    }
                };
                if address.is_none() {
                    set_last_error(memory, teb, 127)?;
                }
                Ok(ptr_result(address))
            },
        ))?;
    }
    {
        let shared = Rc::clone(&shared);
        host.service("kernel32.dll", "DisableThreadLibraryCalls", &[GuestStorage::Pointer], Some(GuestStorage::Int32), Rc::new(
            move |ctx, _, args| {
                let base = pointer(args, 0)?;
                let blocked = {
                    let shared = shared.borrow();
                    match shared.images.iter().find(|image| Some(image.image.base.offset) == base.map(|base| base.offset)) {
                        None => true,
                        Some(image) => image.image.tls.is_some(),
                    }
                };
                if blocked {
                    set_last_error(ctx.memory(), teb, 87)?;
                    return Ok(bool_result(false));
                }
                Ok(bool_result(true))
            },
        ))?;
    }
    for wide in [false, true] {
        let name = if wide { "GetModuleFileNameW" } else { "GetModuleFileNameA" };
        let context = context.clone();
        host.service("kernel32.dll", name, &[GuestStorage::Pointer, GuestStorage::Pointer, GuestStorage::Uint32], Some(GuestStorage::Uint32), Rc::new(
            move |ctx, _, args| {
                let memory = ctx.memory();
                let handle = pointer(args, 0)?;
                let name = match handle {
                    None => Some("quake-typescript.exe".to_string()),
                    Some(handle) => context.library_name(handle),
                };
                let Some(name) = name else {
                    set_last_error(memory, teb, 126)?;
                    return Ok(u32_result(0));
                };
                let size = count(args, 2)?;
                if size == 0 {
                    set_last_error(memory, teb, 122)?;
                    return Ok(u32_result(0));
                }
                let bytes = string_bytes(&name, wide);
                let unit = if wide { 2 } else { 1 };
                let length = bytes.len() / unit - 1;
                let copied = length.min(size - 1);
                let destination = required_pointer(args, 1)?;
                memory.write(destination, &bytes[..copied * unit])?;
                write_unsigned(memory, memory.offset(destination, (copied * unit) as i64)?, unit, 0)?;
                if length >= size {
                    set_last_error(memory, teb, 122)?;
                    return Ok(u32_result(size as u32));
                }
                Ok(u32_result(length as u32))
            },
        ))?;
    }
    install_heap(host, &kernel, &context, process_heap)?;
    install_virtual(host, &kernel)?;
    install_tls(host, &kernel)?;
    install_sync(host, &kernel)?;
    install_time(host, &shared)?;
    {
        let kernel = Rc::clone(&kernel);
        host.service("kernel32.dll", "SetUnhandledExceptionFilter", &[GuestStorage::Pointer], Some(GuestStorage::Pointer), Rc::new(
            move |_, _, args| {
                let mut kernel = kernel.borrow_mut();
                let previous = kernel.exception_filter;
                kernel.exception_filter = pointer(args, 0)?;
                Ok(ptr_result(previous))
            },
        ))?;
    }
    {
        let shared = Rc::clone(&shared);
        host.service("kernel32.dll", "RtlLookupFunctionEntry", &[GuestStorage::Uint64, GuestStorage::Pointer, GuestStorage::Pointer], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let pc = integer(args, 0)? as u64;
                let found = {
                    let shared = shared.borrow();
                    shared.images.iter().find_map(|image| {
                        let base = image.image.base.offset;
                        if pc < base || pc >= base + image.image.byte_length {
                            return None;
                        }
                        let index = image.unwind_records.iter().position(|entry| {
                            pc >= base + u64::from(entry.begin_rva) && pc < base + u64::from(entry.end_rva)
                        })?;
                        Some((image.image.base, image.pe.directories.get(3).map(|table| table.rva), index))
                    })
                };
                let Some((base, directory, index)) = found else {
                    return Ok(ptr_result(None));
                };
                let Some(rva) = directory else {
                    return Err(crate::error::GuestError::callback("Missing PE exception directory"));
                };
                let memory = ctx.memory();
                write_unsigned(memory, required_pointer(args, 1)?, 8, base.offset as i128)?;
                Ok(ptr_result(Some(memory.offset(base, (rva + index as u32 * 12) as i64)?)))
            },
        ))?;
    }
    install_files(host, &kernel, invalid)?;
    install_locale(host)?;
    Ok(())
}

fn is_absolute_path(library: &str) -> bool {
    let bytes = library.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/') {
        return true;
    }
    bytes.len() >= 2 && (bytes[0] == b'\\' || bytes[0] == b'/') && (bytes[1] == b'\\' || bytes[1] == b'/')
}

fn install_heap(
    host: &mut WindowsServiceRegistrar<'_>,
    kernel: &KernelShared,
    context: &WindowsContext,
    process_heap: GuestAddress,
) -> Result<(), GuestError> {
    let teb = host.teb;
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "HeapCreate", &[GuestStorage::Uint32, context.pointer_storage(), context.pointer_storage()], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, _| {
                let memory = ctx.memory();
                let address = memory.allocate(&GuestAllocationOptions {
                    byte_length: 16,
                    alignment: 16,
                    permissions: GuestPermissions::ReadWrite,
                    label: "Windows heap handle".to_string(),
                })?;
                kernel.borrow_mut().heaps.insert(address.offset);
                Ok(ptr_result(Some(address)))
            },
        ))?;
    }
    {
        let kernel = Rc::clone(kernel);
        let context = context.clone();
        host.service("kernel32.dll", "HeapAlloc", &[GuestStorage::Pointer, GuestStorage::Uint32, context.pointer_storage()], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let heap = pointer(args, 0)?;
                if heap.is_none() || !kernel.borrow().heaps.contains(&heap.expect("heap checked").offset) {
                    set_last_error(ctx.memory(), teb, 6)?;
                    return Ok(ptr_result(None));
                }
                let memory = ctx.memory();
                let address = context.allocate(memory, teb, count(args, 2)?, heap.expect("heap checked").offset)?;
                Ok(ptr_result(address))
            },
        ))?;
    }
    {
        let context = context.clone();
        host.service("kernel32.dll", "HeapFree", &[GuestStorage::Pointer, GuestStorage::Uint32, GuestStorage::Pointer], Some(GuestStorage::Int32), Rc::new(
            move |ctx, _, args| {
                let heap = pointer(args, 0)?.map(|heap| heap.offset).unwrap_or(0);
                let memory = ctx.memory();
                Ok(bool_result(context.free(memory, teb, pointer(args, 2)?, heap)?))
            },
        ))?;
    }
    {
        let context = context.clone();
        host.service("kernel32.dll", "HeapReAlloc", &[GuestStorage::Pointer, GuestStorage::Uint32, GuestStorage::Pointer, context.pointer_storage()], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let heap = pointer(args, 0)?;
                let old = pointer(args, 2)?;
                let size = count(args, 3)?;
                let flags = integer(args, 1)? as u32;
                let (Some(heap), Some(old)) = (heap, old) else {
                    set_last_error(ctx.memory(), teb, 87)?;
                    return Ok(ptr_result(None));
                };
                let previous = context.allocation_size(old, heap.offset);
                let Some(previous) = previous else {
                    set_last_error(ctx.memory(), teb, 87)?;
                    return Ok(ptr_result(None));
                };
                if size <= previous {
                    return Ok(ptr_result(Some(old)));
                }
                if (flags & 16) != 0 {
                    return Ok(ptr_result(None));
                }
                let memory = ctx.memory();
                let address = context.allocate(memory, teb, size, heap.offset)?;
                if let Some(address) = address {
                    let bytes = memory.copy(old, previous)?;
                    memory.write(address, &bytes)?;
                    context.free(memory, teb, Some(old), heap.offset)?;
                }
                Ok(ptr_result(address))
            },
        ))?;
    }
    {
        let kernel = Rc::clone(kernel);
        let context = context.clone();
        host.service("kernel32.dll", "HeapDestroy", &[GuestStorage::Pointer], Some(GuestStorage::Int32), Rc::new(
            move |ctx, _, args| {
                let heap = pointer(args, 0)?;
                let Some(heap) = heap else {
                    return Ok(bool_result(false));
                };
                if heap.offset == process_heap.offset || !kernel.borrow_mut().heaps.remove(&heap.offset) {
                    return Ok(bool_result(false));
                }
                let memory = ctx.memory();
                context.destroy_heap(memory, teb, heap.offset)?;
                memory.unmap(heap, 16)?;
                Ok(bool_result(true))
            },
        ))?;
    }
    Ok(())
}

fn install_virtual(host: &mut WindowsServiceRegistrar<'_>, kernel: &KernelShared) -> Result<(), GuestError> {
    let teb = host.teb;
    let pointer_storage = host.pointer_storage();
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "VirtualAlloc", &[GuestStorage::Pointer, pointer_storage, GuestStorage::Uint32, GuestStorage::Uint32], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, context, args| {
                let _ = context;
                let memory = ctx.memory();
                let requested = pointer(args, 0)?;
                let size = count(args, 1)?;
                let flags = integer(args, 2)? as u32;
                let protection = integer(args, 3)? as u32;
                if size == 0 || (flags & !(0x1000 | 0x2000 | 0x100000)) != 0 || ![1, 2, 4, 0x10, 0x20, 0x40].contains(&protection) {
                    return Err(unsupported_windows("kernel32.dll", "VirtualAlloc", "unsupported allocation flags/protection"));
                }
                let permissions = match protection {
                    1 => GuestPermissions::None,
                    2 => GuestPermissions::Read,
                    4 => GuestPermissions::ReadWrite,
                    0x10 => GuestPermissions::Execute,
                    0x20 => GuestPermissions::ReadExecute,
                    _ => GuestPermissions::ReadWriteExecute,
                };
                let mut address = requested;
                let mut bytes = size.div_ceil(4096) * 4096;
                if (flags & 0x2000) != 0 || address.is_none() {
                    if address.is_none() {
                        address = Some(memory.allocate(&GuestAllocationOptions {
                            byte_length: bytes,
                            alignment: 65536,
                            permissions: GuestPermissions::None,
                            label: "Windows VirtualAlloc".to_string(),
                        })?);
                    } else {
                        let requested = address.expect("address checked");
                        let base = requested.offset & !65535;
                        bytes = ((requested.offset - base) as usize + size).div_ceil(4096) * 4096;
                        match memory.map(&GuestMapOptions {
                            base,
                            byte_length: bytes,
                            permissions: GuestPermissions::None,
                            label: "Windows VirtualAlloc".to_string(),
                            bytes: None,
                        }) {
                            Ok(mapped) => address = Some(mapped),
                            Err(_) => {
                                set_last_error(memory, teb, 487)?;
                                return Ok(ptr_result(None));
                            }
                        }
                    }
                    let address = address.expect("reservation checked");
                    kernel.borrow_mut().reservations.insert(address.offset, VirtualReservation { address, size: bytes });
                } else {
                    let requested = address.expect("address checked");
                    let base = requested.offset & !4095;
                    bytes = ((requested.offset - base) as usize + size).div_ceil(4096) * 4096;
                    address = memory.pointer(base)?;
                    let inside = address.is_some()
                        && kernel.borrow().reservations.values().any(|region| {
                            base >= region.address.offset
                                && base + bytes as u64 <= region.address.offset + region.size as u64
                        });
                    if !inside {
                        set_last_error(memory, teb, 487)?;
                        return Ok(ptr_result(None));
                    }
                }
                let address = address.expect("address checked");
                if (flags & 0x1000) != 0 {
                    memory.protect(address, bytes, permissions)?;
                }
                Ok(ptr_result(Some(address)))
            },
        ))?;
    }
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "VirtualFree", &[GuestStorage::Pointer, pointer_storage, GuestStorage::Uint32], Some(GuestStorage::Int32), Rc::new(
            move |ctx, _, args| {
                let memory = ctx.memory();
                let address = required_pointer(args, 0)?;
                let size = count(args, 1)?;
                let flags = integer(args, 2)? as u32;
                if flags == 0x8000 && size == 0 {
                    let region = kernel.borrow().reservations.get(&address.offset).map(|region| (region.address, region.size));
                    let Some((base, region_size)) = region else {
                        return Ok(bool_result(false));
                    };
                    memory.unmap(base, region_size)?;
                    kernel.borrow_mut().reservations.remove(&address.offset);
                    return Ok(bool_result(true));
                }
                if flags == 0x4000 && size > 0 {
                    let inside = kernel.borrow().reservations.values().any(|region| {
                        address.offset >= region.address.offset && address.offset + size as u64 <= region.address.offset + region.size as u64
                    });
                    if !inside {
                        return Ok(bool_result(false));
                    }
                    memory.protect(address, size, GuestPermissions::ReadWrite)?;
                    memory.write(address, &vec![0u8; size])?;
                    memory.protect(address, size, GuestPermissions::None)?;
                    return Ok(bool_result(true));
                }
                set_last_error(memory, teb, 87)?;
                Ok(bool_result(false))
            },
        ))?;
    }
    Ok(())
}

fn install_tls(host: &mut WindowsServiceRegistrar<'_>, kernel: &KernelShared) -> Result<(), GuestError> {
    let teb = host.teb;
    let width = host.pointer_bytes;
    let dynamic_tls = host.memory.allocate(&GuestAllocationOptions {
        byte_length: 1024 * width,
        alignment: 16,
        permissions: GuestPermissions::ReadWrite,
        label: "Windows expansion TLS slots".to_string(),
    })?;
    write_pointer(
        host.memory,
        host.memory.offset(teb, if width == 4 { 0xf94 } else { 0x1780 })?,
        Some(dynamic_tls),
    )?;
    let slot_base = if width == 4 { 0xe10 } else { 0x1480 };
    let tls_slot = move |memory: &SparseGuestMemory, index: u32| -> Result<GuestAddress, GuestError> {
        if index < 64 {
            memory.offset(teb, slot_base + index as i64 * width as i64)
        } else {
            memory.offset(dynamic_tls, (index - 64) as i64 * width as i64)
        }
    };
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "TlsAlloc", &[], Some(GuestStorage::Uint32), Rc::new(
            move |ctx, _, _| {
                for index in 0..1088 {
                    if kernel.borrow().tls_slots.contains(&index) {
                        continue;
                    }
                    kernel.borrow_mut().tls_slots.insert(index);
                    let memory = ctx.memory();
                    write_unsigned(memory, tls_slot(memory, index)?, width, 0)?;
                    return Ok(u32_result(index));
                }
                Ok(u32_result(0xffff_ffff))
            },
        ))?;
    }
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "TlsFree", &[GuestStorage::Uint32], Some(GuestStorage::Int32), Rc::new(
            move |ctx, _, args| {
                let index = integer(args, 0)? as u32;
                if !kernel.borrow_mut().tls_slots.remove(&index) {
                    set_last_error(ctx.memory(), teb, 87)?;
                    return Ok(bool_result(false));
                }
                Ok(bool_result(true))
            },
        ))?;
    }
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "TlsGetValue", &[GuestStorage::Uint32], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let index = integer(args, 0)? as u32;
                if !kernel.borrow().tls_slots.contains(&index) {
                    set_last_error(ctx.memory(), teb, 87)?;
                    return Ok(ptr_result(None));
                }
                let memory = ctx.memory();
                set_last_error(memory, teb, 0)?;
                let slot = tls_slot(memory, index)?;
                Ok(ptr_result(memory.read_pointer(slot)?))
            },
        ))?;
    }
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "TlsSetValue", &[GuestStorage::Uint32, GuestStorage::Pointer], Some(GuestStorage::Int32), Rc::new(
            move |ctx, _, args| {
                let index = integer(args, 0)? as u32;
                if !kernel.borrow().tls_slots.contains(&index) {
                    set_last_error(ctx.memory(), teb, 87)?;
                    return Ok(bool_result(false));
                }
                let value = pointer(args, 1)?;
                let memory = ctx.memory();
                let slot = tls_slot(memory, index)?;
                write_pointer(memory, slot, value)?;
                Ok(bool_result(true))
            },
        ))?;
    }
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "FlsAlloc", &[GuestStorage::Pointer], Some(GuestStorage::Uint32), Rc::new(
            move |ctx, _, args| {
                let callback = pointer(args, 0)?;
                for index in 0..128 {
                    if kernel.borrow().fls.contains_key(&index) {
                        continue;
                    }
                    kernel.borrow_mut().fls.insert(index, FlsSlot { callback, value: None });
                    return Ok(u32_result(index));
                }
                set_last_error(ctx.memory(), teb, 8)?;
                Ok(u32_result(0xffff_ffff))
            },
        ))?;
    }
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "FlsGetValue", &[GuestStorage::Uint32], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let index = integer(args, 0)? as u32;
                let value = kernel.borrow().fls.get(&index).map(|slot| slot.value);
                let Some(value) = value else {
                    set_last_error(ctx.memory(), teb, 87)?;
                    return Ok(ptr_result(None));
                };
                set_last_error(ctx.memory(), teb, 0)?;
                Ok(ptr_result(value))
            },
        ))?;
    }
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "FlsSetValue", &[GuestStorage::Uint32, GuestStorage::Pointer], Some(GuestStorage::Int32), Rc::new(
            move |ctx, _, args| {
                let index = integer(args, 0)? as u32;
                let value = pointer(args, 1)?;
                if !kernel.borrow().fls.contains_key(&index) {
                    set_last_error(ctx.memory(), teb, 87)?;
                    return Ok(bool_result(false));
                }
                if let Some(slot) = kernel.borrow_mut().fls.get_mut(&index) {
                    slot.value = value;
                }
                Ok(bool_result(true))
            },
        ))?;
    }
    {
        let kernel = Rc::clone(kernel);
        let shared = Rc::clone(&host.shared);
        host.service("kernel32.dll", "FlsFree", &[GuestStorage::Uint32], Some(GuestStorage::Int32), Rc::new(
            move |ctx, context, args| {
                let index = integer(args, 0)? as u32;
                let slot = kernel.borrow_mut().fls.remove(&index);
                let Some(slot) = slot else {
                    set_last_error(ctx.memory(), teb, 87)?;
                    return Ok(bool_result(false));
                };
                if let (Some(callback), Some(value)) = (slot.callback, slot.value) {
                    invoke_nested(
                        ctx,
                        &shared,
                        width,
                        context,
                        callback,
                        &[GuestStorage::Pointer],
                        None,
                        vec![GuestCallValue::Pointer(Some(value))],
                        true,
                    )?;
                }
                Ok(bool_result(true))
            },
        ))?;
    }
    Ok(())
}

fn initialize_lock(
    memory: &mut SparseGuestMemory,
    kernel: &KernelShared,
    width: usize,
    address: GuestAddress,
    spin: i128,
) -> Result<(), GuestError> {
    memory.write(address, &vec![0u8; if width == 4 { 24 } else { 40 }])?;
    write_unsigned(memory, memory.offset(address, width as i64)?, 4, 0xffff_ffff)?;
    write_unsigned(
        memory,
        memory.offset(address, if width == 4 { 20 } else { 32 })?,
        width,
        spin & 0x7fff_ffff,
    )?;
    kernel.borrow_mut().locks.insert(address.offset);
    Ok(())
}

fn install_sync(host: &mut WindowsServiceRegistrar<'_>, kernel: &KernelShared) -> Result<(), GuestError> {
    let teb = host.teb;
    let width = host.pointer_bytes;
    let thread_id = host.thread_id;
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "InitializeCriticalSection", &[GuestStorage::Pointer], None, Rc::new(
            move |ctx, _, args| {
                initialize_lock(ctx.memory(), &kernel, width, required_pointer(args, 0)?, 0)?;
                Ok(void_result())
            },
        ))?;
    }
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "InitializeCriticalSectionAndSpinCount", &[GuestStorage::Pointer, GuestStorage::Uint32], Some(GuestStorage::Int32), Rc::new(
            move |ctx, _, args| {
                let spin = integer(args, 1)?;
                initialize_lock(ctx.memory(), &kernel, width, required_pointer(args, 0)?, spin)?;
                Ok(bool_result(true))
            },
        ))?;
    }
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "InitializeCriticalSectionEx", &[GuestStorage::Pointer, GuestStorage::Uint32, GuestStorage::Uint32], Some(GuestStorage::Int32), Rc::new(
            move |ctx, _, args| {
                if (integer(args, 2)? & !0x0100_0000) != 0 {
                    set_last_error(ctx.memory(), teb, 87)?;
                    return Ok(bool_result(false));
                }
                let spin = integer(args, 1)?;
                initialize_lock(ctx.memory(), &kernel, width, required_pointer(args, 0)?, spin)?;
                Ok(bool_result(true))
            },
        ))?;
    }
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "SetCriticalSectionSpinCount", &[GuestStorage::Pointer, GuestStorage::Uint32], Some(GuestStorage::Uint32), Rc::new(
            move |ctx, _, args| {
                let address = required_pointer(args, 0)?;
                if !kernel.borrow().locks.contains(&address.offset) {
                    return Err(GuestError::callback("Uninitialized guest critical section"));
                }
                let memory = ctx.memory();
                let at = memory.offset(address, if width == 4 { 20 } else { 32 })?;
                let previous = read_unsigned(memory, at, width)?;
                write_unsigned(memory, at, width, integer(args, 1)? & 0x7fff_ffff)?;
                Ok(u32_result(previous as u32))
            },
        ))?;
    }
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "DeleteCriticalSection", &[GuestStorage::Pointer], None, Rc::new(
            move |_, _, args| {
                kernel.borrow_mut().locks.remove(&required_pointer(args, 0)?.offset);
                Ok(void_result())
            },
        ))?;
    }
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "EnterCriticalSection", &[GuestStorage::Pointer], None, Rc::new(
            move |ctx, context, args| {
                let _ = context;
                let address = required_pointer(args, 0)?;
                if !kernel.borrow().locks.contains(&address.offset) {
                    return Err(GuestError::callback("Uninitialized guest critical section"));
                }
                let memory = ctx.memory();
                let depth = read_unsigned(memory, memory.offset(address, (width + 4) as i64)?, 4)?;
                let owner = read_unsigned(memory, memory.offset(address, (width + 8) as i64)?, width)?;
                if depth != 0 && owner != thread_id as u64 {
                    return Err(unsupported_windows(
                        "kernel32.dll",
                        "EnterCriticalSection",
                        "contended lock requires a guest thread scheduler",
                    ));
                }
                write_unsigned(memory, memory.offset(address, width as i64)?, 4, depth as i128)?;
                write_unsigned(memory, memory.offset(address, (width + 4) as i64)?, 4, (depth + 1) as i128)?;
                write_unsigned(memory, memory.offset(address, (width + 8) as i64)?, width, thread_id as i128)?;
                Ok(void_result())
            },
        ))?;
    }
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "LeaveCriticalSection", &[GuestStorage::Pointer], None, Rc::new(
            move |ctx, _, args| {
                let address = required_pointer(args, 0)?;
                let memory = ctx.memory();
                let depth = read_unsigned(memory, memory.offset(address, (width + 4) as i64)?, 4)?;
                let owner = read_unsigned(memory, memory.offset(address, (width + 8) as i64)?, width)?;
                if !kernel.borrow().locks.contains(&address.offset) || depth == 0 || owner != thread_id as u64 {
                    return Err(GuestError::callback("Unowned guest critical section"));
                }
                write_unsigned(memory, memory.offset(address, width as i64)?, 4, depth as i128 - 2)?;
                write_unsigned(memory, memory.offset(address, (width + 4) as i64)?, 4, depth as i128 - 1)?;
                if depth == 1 {
                    write_unsigned(memory, memory.offset(address, (width + 8) as i64)?, width, 0)?;
                }
                Ok(void_result())
            },
        ))?;
    }
    for (name, change) in [("InterlockedIncrement", 1i128), ("InterlockedDecrement", -1i128)] {
        host.service("kernel32.dll", name, &[GuestStorage::Pointer], Some(GuestStorage::Int32), Rc::new(
            move |ctx, _, args| {
                let memory = ctx.memory();
                let address = required_pointer(args, 0)?;
                let value = ((read_unsigned(memory, address, 4)? as i128 + change) & 0xffff_ffff) as u32 as i32;
                write_unsigned(memory, address, 4, value as i128 & 0xffff_ffff)?;
                Ok(GuestCallResult::Value(GuestCallValue::Int32(value)))
            },
        ))?;
    }
    host.service("kernel32.dll", "AcquireSRWLockExclusive", &[GuestStorage::Pointer], None, Rc::new(
        move |ctx, context, args| {
            let _ = context;
            let memory = ctx.memory();
            let address = required_pointer(args, 0)?;
            if read_unsigned(memory, address, width)? != 0 {
                return Err(unsupported_windows(
                    "kernel32.dll",
                    "AcquireSRWLockExclusive",
                    "contended lock requires a guest thread scheduler",
                ));
            }
            write_unsigned(memory, address, width, 1)?;
            Ok(void_result())
        },
    ))?;
    host.service("kernel32.dll", "ReleaseSRWLockExclusive", &[GuestStorage::Pointer], None, Rc::new(
        move |ctx, _, args| {
            let memory = ctx.memory();
            let address = required_pointer(args, 0)?;
            if read_unsigned(memory, address, width)? == 0 {
                return Err(GuestError::callback("Unowned guest SRW lock"));
            }
            write_unsigned(memory, address, width, 0)?;
            Ok(void_result())
        },
    ))?;
    host.service("kernel32.dll", "InitializeSListHead", &[GuestStorage::Pointer], None, Rc::new(
        move |ctx, _, args| {
            let memory = ctx.memory();
            memory.write(required_pointer(args, 0)?, &vec![0u8; if width == 4 { 8 } else { 16 }])?;
            Ok(void_result())
        },
    ))?;
    host.service("kernel32.dll", "InterlockedFlushSList", &[GuestStorage::Pointer], Some(GuestStorage::Pointer), Rc::new(
        move |ctx, _, args| {
            let memory = ctx.memory();
            let address = required_pointer(args, 0)?;
            if address.offset % (if width == 4 { 8 } else { 16 }) != 0 {
                return Err(GuestError::invalid("Unaligned Windows SLIST_HEADER"));
            }
            if width == 4 {
                let next = memory.read_pointer(address)?;
                let Some(next) = next else {
                    return Ok(ptr_result(None));
                };
                let sequence = read_unsigned(memory, memory.offset(address, 6)?, 2)?;
                memory.write(address, &[0u8; 8])?;
                write_unsigned(memory, memory.offset(address, 6)?, 2, sequence as i128 + 1)?;
                return Ok(ptr_result(Some(next)));
            }
            let lower = read_unsigned(memory, address, 8)?;
            let upper_at = memory.offset(address, 8)?;
            let upper = read_unsigned(memory, upper_at, 8)?;
            let next = memory.pointer(upper & !15)?;
            let Some(next) = next else {
                return Ok(ptr_result(None));
            };
            write_unsigned(memory, address, 8, ((lower & !65535) + 65536) as i128)?;
            write_unsigned(memory, upper_at, 8, (upper & 15) as i128)?;
            Ok(ptr_result(Some(next)))
        },
    ))?;
    host.service("kernel32.dll", "WakeAllConditionVariable", &[GuestStorage::Pointer], None, Rc::new(
        move |_, _, _| Ok(void_result()),
    ))?;
    host.service("kernel32.dll", "IsDebuggerPresent", &[], Some(GuestStorage::Int32), Rc::new(
        move |_, _, _| Ok(bool_result(false)),
    ))?;
    host.service("kernel32.dll", "IsProcessorFeaturePresent", &[GuestStorage::Uint32], Some(GuestStorage::Int32), Rc::new(
        move |_, _, args| Ok(bool_result([6, 10].contains(&(integer(args, 0)? as i32)))),
    ))?;
    Ok(())
}

fn install_time(host: &mut WindowsServiceRegistrar<'_>, shared: &SharedWindows) -> Result<(), GuestError> {
    {
        let shared = Rc::clone(shared);
        host.service("kernel32.dll", "GetSystemTimeAsFileTime", &[GuestStorage::Pointer], None, Rc::new(
            move |ctx, _, args| {
                let memory = ctx.memory();
                let ticks = now_millis(&shared) as i128 * 10_000 + 116_444_736_000_000_000;
                write_unsigned(memory, required_pointer(args, 0)?, 8, ticks)?;
                Ok(void_result())
            },
        ))?;
    }
    {
        let shared = Rc::clone(shared);
        host.service("kernel32.dll", "QueryPerformanceCounter", &[GuestStorage::Pointer], Some(GuestStorage::Int32), Rc::new(
            move |ctx, _, args| {
                let value = shared
                    .borrow()
                    .capabilities
                    .performance_counter
                    .clone()
                    .map(|counter| counter())
                    .unwrap_or_else(performance_micros);
                write_unsigned(ctx.memory(), required_pointer(args, 0)?, 8, value as i128)?;
                Ok(bool_result(true))
            },
        ))?;
    }
    {
        let shared = Rc::clone(shared);
        host.service("kernel32.dll", "QueryPerformanceFrequency", &[GuestStorage::Pointer], Some(GuestStorage::Int32), Rc::new(
            move |ctx, _, args| {
                let value = shared.borrow().capabilities.performance_frequency.unwrap_or(1_000_000);
                write_unsigned(ctx.memory(), required_pointer(args, 0)?, 8, value as i128)?;
                Ok(bool_result(true))
            },
        ))?;
    }
    for name in ["GetSystemTime", "GetLocalTime"] {
        let local = name == "GetLocalTime";
        let shared = Rc::clone(shared);
        host.service("kernel32.dll", name, &[GuestStorage::Pointer], None, Rc::new(
            move |ctx, _, args| {
                let now = now_millis(&shared);
                let offset = if local { local_offset_at(now / 1000).0 } else { 0 };
                let fields = system_time_fields(now, offset);
                let memory = ctx.memory();
                let address = required_pointer(args, 0)?;
                for (index, value) in fields.iter().enumerate() {
                    write_unsigned(memory, memory.offset(address, index as i64 * 2)?, 2, *value as i128)?;
                }
                Ok(void_result())
            },
        ))?;
    }
    {
        let shared = Rc::clone(shared);
        host.service("kernel32.dll", "GetTimeZoneInformation", &[GuestStorage::Pointer], Some(GuestStorage::Uint32), Rc::new(
            move |ctx, _, args| {
                let memory = ctx.memory();
                let address = required_pointer(args, 0)?;
                memory.write(address, &[0u8; 172])?;
                let (offset, _) = local_offset_at(now_millis(&shared) / 1000);
                write_unsigned(memory, address, 4, (-(offset / 60)) as i128)?;
                Ok(u32_result(0))
            },
        ))?;
    }
    Ok(())
}

fn install_files(
    host: &mut WindowsServiceRegistrar<'_>,
    kernel: &KernelShared,
    invalid: Option<GuestAddress>,
) -> Result<(), GuestError> {
    let teb = host.teb;
    let width = host.pointer_bytes;
    for (id, stream, label) in [(-10, FileKind::Stdin, "stdin"), (-11, FileKind::Stdout, "stdout"), (-12, FileKind::Stderr, "stderr")] {
        let address = stored_string(host.memory, label, false)?;
        let mut kernel = kernel.borrow_mut();
        kernel.standards.insert(id, address);
        kernel.handles.insert(address.offset, FileHandle { file: stream, offset: 0 });
    }
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "GetStdHandle", &[GuestStorage::Int32], Some(GuestStorage::Pointer), Rc::new(
            move |_, _, args| {
                let value = kernel.borrow().standards.get(&(integer(args, 0)? as i32)).copied().or(invalid);
                Ok(ptr_result(value))
            },
        ))?;
    }
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "SetStdHandle", &[GuestStorage::Int32, GuestStorage::Pointer], Some(GuestStorage::Int32), Rc::new(
            move |_, _, args| {
                let Some(address) = pointer(args, 1)? else {
                    return Ok(bool_result(false));
                };
                kernel.borrow_mut().standards.insert(integer(args, 0)? as i32, address);
                Ok(bool_result(true))
            },
        ))?;
    }
    host.service("kernel32.dll", "SetHandleCount", &[GuestStorage::Uint32], Some(GuestStorage::Uint32), Rc::new(
        move |_, _, args| Ok(u32_result(integer(args, 0)? as u32)),
    ))?;
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "GetFileType", &[GuestStorage::Pointer], Some(GuestStorage::Uint32), Rc::new(
            move |_, _, args| {
                let value = match pointer(args, 0)? {
                    None => 0,
                    Some(address) => match kernel.borrow().handles.get(&address.offset) {
                        None => 0,
                        Some(handle) => match handle.file {
                            FileKind::File(_) => 1,
                            _ => 2,
                        },
                    },
                };
                Ok(u32_result(value))
            },
        ))?;
    }
    for name in ["GetStartupInfoA", "GetStartupInfoW"] {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", name, &[GuestStorage::Pointer], None, Rc::new(
            move |ctx, _, args| {
                let memory = ctx.memory();
                let address = required_pointer(args, 0)?;
                let size = if width == 4 { 68 } else { 104 };
                memory.write(address, &vec![0u8; size])?;
                write_unsigned(memory, address, 4, size as i128)?;
                for (index, id) in [-10, -11, -12].iter().enumerate() {
                    let slot = memory.offset(address, ((if width == 4 { 56 } else { 80 }) + index * width) as i64)?;
                    write_pointer(memory, slot, kernel.borrow().standards.get(id).copied())?;
                }
                Ok(void_result())
            },
        ))?;
    }
    {
        let shared = Rc::clone(&host.shared);
        let kernel = Rc::clone(kernel);
        host.service(
            "kernel32.dll",
            "CreateFileA",
            &[GuestStorage::Pointer, GuestStorage::Uint32, GuestStorage::Uint32, GuestStorage::Pointer, GuestStorage::Uint32, GuestStorage::Uint32, GuestStorage::Pointer],
            Some(GuestStorage::Pointer),
            Rc::new(move |ctx, _, args| {
                let memory = ctx.memory();
                let access = integer(args, 1)? as u32;
                let creation = integer(args, 4)? as u32;
                let path = read_string(memory, required_pointer(args, 0)?, false)?;
                let file = shared
                    .borrow()
                    .capabilities
                    .open_file
                    .clone()
                    .and_then(|open| {
                        open(
                            &path,
                            WindowsOpenOptions {
                                read: (access & 0x8000_0000) != 0,
                                write: (access & 0x4000_0000) != 0,
                                creation,
                            },
                        )
                    });
                let Some(file) = file else {
                    set_last_error(memory, teb, 2)?;
                    return Ok(ptr_result(invalid));
                };
                let address = memory.allocate(&GuestAllocationOptions {
                    byte_length: 16,
                    alignment: 16,
                    permissions: GuestPermissions::ReadWrite,
                    label: "Windows file handle".to_string(),
                })?;
                kernel.borrow_mut().handles.insert(address.offset, FileHandle { file: FileKind::File(file), offset: 0 });
                Ok(ptr_result(Some(address)))
            }),
        )?;
    }
    {
        let shared = Rc::clone(&host.shared);
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "ReadFile", &[GuestStorage::Pointer, GuestStorage::Pointer, GuestStorage::Uint32, GuestStorage::Pointer, GuestStorage::Pointer], Some(GuestStorage::Int32), Rc::new(
            move |ctx, context, args| {
                let _ = context;
                let handle = pointer(args, 0)?;
                let Some(handle) = handle else {
                    set_last_error(ctx.memory(), teb, 6)?;
                    return Ok(bool_result(false));
                };
                if !kernel.borrow().handles.contains_key(&handle.offset) {
                    set_last_error(ctx.memory(), teb, 6)?;
                    return Ok(bool_result(false));
                }
                if pointer(args, 4)? != None {
                    return Err(unsupported_windows("kernel32.dll", "ReadFile", "overlapped I/O is not implemented"));
                }
                let length = count(args, 2)?;
                enum ReadOutcome {
                    Bytes(Vec<u8>),
                    NoAccess,
                }
                let outcome = {
                    let mut kernel = kernel.borrow_mut();
                    let handle = kernel.handles.get_mut(&handle.offset).ok_or_else(|| GuestError::callback("Windows handle vanished"))?;
                    match &mut handle.file {
                        FileKind::Stdin => ReadOutcome::Bytes(
                            shared.borrow().capabilities.standard_input.clone().map(|input| input(length)).unwrap_or_default(),
                        ),
                        FileKind::Stdout | FileKind::Stderr => ReadOutcome::NoAccess,
                        FileKind::File(file) => ReadOutcome::Bytes(file.read(handle.offset, length)),
                    }
                };
                let ReadOutcome::Bytes(bytes) = outcome else {
                    set_last_error(ctx.memory(), teb, 5)?;
                    return Ok(bool_result(false));
                };
                if bytes.len() > length {
                    return Err(GuestError::callback("File capability exceeded requested read length"));
                }
                let memory = ctx.memory();
                memory.write(required_pointer(args, 1)?, &bytes)?;
                kernel.borrow_mut().handles.get_mut(&handle.offset).map(|handle| handle.offset += bytes.len());
                if let Some(read) = pointer(args, 3)? {
                    write_unsigned(memory, read, 4, bytes.len() as i128)?;
                }
                Ok(bool_result(true))
            },
        ))?;
    }
    {
        let shared = Rc::clone(&host.shared);
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "WriteFile", &[GuestStorage::Pointer, GuestStorage::Pointer, GuestStorage::Uint32, GuestStorage::Pointer, GuestStorage::Pointer], Some(GuestStorage::Int32), Rc::new(
            move |ctx, context, args| {
                let _ = context;
                let handle = pointer(args, 0)?;
                let Some(handle) = handle else {
                    set_last_error(ctx.memory(), teb, 6)?;
                    return Ok(bool_result(false));
                };
                if !kernel.borrow().handles.contains_key(&handle.offset) {
                    set_last_error(ctx.memory(), teb, 6)?;
                    return Ok(bool_result(false));
                }
                if pointer(args, 4)? != None {
                    return Err(unsupported_windows("kernel32.dll", "WriteFile", "overlapped I/O is not implemented"));
                }
                let memory = ctx.memory();
                let bytes = memory.copy(required_pointer(args, 1)?, count(args, 2)?)?;
                enum WriteOutcome {
                    Written(usize),
                    NoOutput,
                    NoAccess,
                }
                let outcome = {
                    let mut kernel = kernel.borrow_mut();
                    let handle = kernel.handles.get_mut(&handle.offset).ok_or_else(|| GuestError::callback("Windows handle vanished"))?;
                    match &mut handle.file {
                        FileKind::Stdout | FileKind::Stderr => {
                            let stream = if matches!(handle.file, FileKind::Stdout) {
                                WindowsStream::Stdout
                            } else {
                                WindowsStream::Stderr
                            };
                            match shared.borrow().capabilities.standard_output.clone() {
                                None => WriteOutcome::NoOutput,
                                Some(output) => {
                                    output(stream, &bytes);
                                    WriteOutcome::Written(bytes.len())
                                }
                            }
                        }
                        FileKind::Stdin => WriteOutcome::NoAccess,
                        FileKind::File(file) => WriteOutcome::Written(file.write(handle.offset, &bytes)),
                    }
                };
                let written = match outcome {
                    WriteOutcome::Written(written) => written,
                    WriteOutcome::NoOutput => {
                        set_last_error(ctx.memory(), teb, 6)?;
                        return Ok(bool_result(false));
                    }
                    WriteOutcome::NoAccess => {
                        set_last_error(ctx.memory(), teb, 5)?;
                        return Ok(bool_result(false));
                    }
                };
                kernel.borrow_mut().handles.get_mut(&handle.offset).map(|handle| handle.offset += written);
                if let Some(out) = pointer(args, 3)? {
                    write_unsigned(ctx.memory(), out, 4, written as i128)?;
                }
                Ok(bool_result(true))
            },
        ))?;
    }
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "CloseHandle", &[GuestStorage::Pointer], Some(GuestStorage::Int32), Rc::new(
            move |ctx, _, args| {
                let address = pointer(args, 0)?;
                let Some(address) = address else {
                    set_last_error(ctx.memory(), teb, 6)?;
                    return Ok(bool_result(false));
                };
                let mut kernel = kernel.borrow_mut();
                let Some(mut handle) = kernel.handles.remove(&address.offset) else {
                    set_last_error(ctx.memory(), teb, 6)?;
                    return Ok(bool_result(false));
                };
                if let FileKind::File(file) = &mut handle.file {
                    file.close();
                }
                Ok(bool_result(true))
            },
        ))?;
    }
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "FlushFileBuffers", &[GuestStorage::Pointer], Some(GuestStorage::Int32), Rc::new(
            move |_, _, args| {
                let mut kernel = kernel.borrow_mut();
                let Some(handle) = pointer(args, 0)?.and_then(|address| kernel.handles.get_mut(&address.offset)) else {
                    return Ok(bool_result(false));
                };
                if let FileKind::File(file) = &mut handle.file {
                    file.flush();
                }
                Ok(bool_result(true))
            },
        ))?;
    }
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "SetFilePointer", &[GuestStorage::Pointer, GuestStorage::Int32, GuestStorage::Pointer, GuestStorage::Uint32], Some(GuestStorage::Uint32), Rc::new(
            move |ctx, _, args| {
                let memory = ctx.memory();
                let address = pointer(args, 0)?;
                let mut kernel = kernel.borrow_mut();
                let handle = address.and_then(|address| kernel.handles.get_mut(&address.offset));
                let Some(handle) = handle else {
                    set_last_error(memory, teb, 6)?;
                    return Ok(u32_result(0xffff_ffff));
                };
                let FileKind::File(file) = &mut handle.file else {
                    set_last_error(memory, teb, 6)?;
                    return Ok(u32_result(0xffff_ffff));
                };
                let high = pointer(args, 2)?;
                let low = integer(args, 1)? as u32 as i32 as i64;
                let distance = match high {
                    None => low as i128,
                    Some(high) => {
                        ((read_unsigned(memory, high, 4)? as u32 as i32 as i64) as i128) << 32
                            | (low as u32) as i128
                    }
                };
                let origin = integer(args, 3)? as u32;
                let base: i128 = match origin {
                    0 => 0,
                    1 => handle.offset as i128,
                    2 => file.size() as i128,
                    _ => {
                        set_last_error(memory, teb, 87)?;
                        return Ok(u32_result(0xffff_ffff));
                    }
                };
                let position = base + distance;
                if origin > 2 || position < 0 || position > (1i128 << 53) - 1 {
                    set_last_error(memory, teb, 87)?;
                    return Ok(u32_result(0xffff_ffff));
                }
                handle.offset = position as usize;
                if let Some(high) = high {
                    write_unsigned(memory, high, 4, position >> 32)?;
                }
                Ok(u32_result((position & 0xffff_ffff) as u32))
            },
        ))?;
    }
    {
        let kernel = Rc::clone(kernel);
        host.service("kernel32.dll", "SetEndOfFile", &[GuestStorage::Pointer], Some(GuestStorage::Int32), Rc::new(
            move |_, _, args| {
                let mut kernel = kernel.borrow_mut();
                let handle = pointer(args, 0)?.and_then(|address| kernel.handles.get_mut(&address.offset));
                let Some(handle) = handle else {
                    return Ok(bool_result(false));
                };
                let FileKind::File(file) = &mut handle.file else {
                    return Ok(bool_result(false));
                };
                let offset = handle.offset;
                file.truncate(offset);
                Ok(bool_result(true))
            },
        ))?;
    }
    Ok(())
}

/// Windows-1252 byte-to-code-point table.
fn cp1252_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    for (index, slot) in table.iter_mut().enumerate() {
        *slot = index as u32;
    }
    let high = [
        0x20ac, 0xfffd, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x02c6, 0x2030, 0x0160,
        0x2039, 0x0152, 0xfffd, 0x017d, 0xfffd, 0xfffd, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022,
        0x2013, 0x2014, 0x02dc, 0x2122, 0x0161, 0x203a, 0x0153, 0xfffd, 0x017e, 0x0178,
    ];
    for (index, code) in high.iter().enumerate() {
        table[0x80 + index] = *code;
    }
    table
}

fn string_input(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
    length: i64,
    wide: bool,
) -> Result<String, GuestError> {
    if length == -1 {
        return Ok(format!("{}\0", read_string(memory, address, wide)?));
    }
    if length < 0 || length > 1024 * 1024 {
        return Err(GuestError::invalid("Invalid Windows string length"));
    }
    let mut result = String::new();
    for index in 0..length {
        let unit = if wide {
            u32::from(memory.read_u16(memory.offset(address, index * 2)?)?)
        } else {
            u32::from(memory.read_u8(memory.offset(address, index)?)?)
        };
        result.push(char::from_u32(unit).unwrap_or(char::REPLACEMENT_CHARACTER));
    }
    Ok(result)
}

fn is_js_space(value: char) -> bool {
    matches!(
        value,
        '\u{0009}' | '\u{000a}' | '\u{000b}' | '\u{000c}' | '\u{000d}' | '\u{0020}' | '\u{00a0}'
            | '\u{1680}' | '\u{2000}' | '\u{2001}' | '\u{2002}' | '\u{2003}' | '\u{2004}'
            | '\u{2005}' | '\u{2006}' | '\u{2007}' | '\u{2008}' | '\u{2009}' | '\u{200a}'
            | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

fn install_locale(host: &mut WindowsServiceRegistrar<'_>) -> Result<(), GuestError> {
    let teb = host.teb;
    let cp1252 = cp1252_table();
    let mut cp1252_reverse: HashMap<u32, u8> = HashMap::new();
    for (byte, code) in cp1252.iter().enumerate() {
        cp1252_reverse.insert(*code, byte as u8);
    }
    {
        let cp1252 = cp1252;
        host.service("kernel32.dll", "MultiByteToWideChar", &[GuestStorage::Uint32, GuestStorage::Uint32, GuestStorage::Pointer, GuestStorage::Int32, GuestStorage::Pointer, GuestStorage::Int32], Some(GuestStorage::Int32), Rc::new(
            move |ctx, context, args| {
                let _ = context;
                let codepage = integer(args, 0)? as i32;
                if ![0, 1252, 65001].contains(&codepage) {
                    return Err(unsupported_windows("kernel32.dll", "MultiByteToWideChar", format!("code page {codepage}")));
                }
                let memory = ctx.memory();
                let source = string_input(memory, required_pointer(args, 2)?, integer(args, 3)? as i64, false)?;
                let bytes: Vec<u8> = source.chars().map(|c| c as u32 as u8).collect();
                let text = if codepage == 65001 {
                    String::from_utf8_lossy(&bytes).into_owned()
                } else {
                    bytes.iter().map(|byte| char::from_u32(cp1252[*byte as usize]).unwrap_or(char::REPLACEMENT_CHARACTER)).collect()
                };
                let utf16: Vec<u16> = text.encode_utf16().collect();
                let capacity = integer(args, 5)? as i64;
                let output = pointer(args, 4)?;
                if capacity != 0 {
                    if output.is_none() || capacity < utf16.len() as i64 {
                        set_last_error(memory, teb, 122)?;
                        return Ok(GuestCallResult::Value(GuestCallValue::Int32(0)));
                    }
                    let mut encoded = vec![0u8; utf16.len() * 2];
                    for (index, unit) in utf16.iter().enumerate() {
                        encoded[index * 2] = (unit & 0xff) as u8;
                        encoded[index * 2 + 1] = (unit >> 8) as u8;
                    }
                    memory.write(output.expect("output checked"), &encoded)?;
                }
                Ok(GuestCallResult::Value(GuestCallValue::Int32(utf16.len() as i32)))
            },
        ))?;
    }
    {
        let cp1252_reverse = cp1252_reverse;
        host.service("kernel32.dll", "WideCharToMultiByte", &[GuestStorage::Uint32, GuestStorage::Uint32, GuestStorage::Pointer, GuestStorage::Int32, GuestStorage::Pointer, GuestStorage::Int32, GuestStorage::Pointer, GuestStorage::Pointer], Some(GuestStorage::Int32), Rc::new(
            move |ctx, context, args| {
                let _ = context;
                let codepage = integer(args, 0)? as i32;
                if ![0, 1252, 65001].contains(&codepage) {
                    return Err(unsupported_windows("kernel32.dll", "WideCharToMultiByte", format!("code page {codepage}")));
                }
                let memory = ctx.memory();
                let source = string_input(memory, required_pointer(args, 2)?, integer(args, 3)? as i64, true)?;
                let mut used_default = false;
                let bytes = if codepage == 65001 {
                    source.as_bytes().to_vec()
                } else {
                    source
                        .encode_utf16()
                        .map(|unit| match cp1252_reverse.get(&(unit as u32)) {
                            Some(byte) => *byte,
                            None => {
                                used_default = true;
                                63
                            }
                        })
                        .collect::<Vec<_>>()
                };
                let capacity = integer(args, 5)? as i64;
                let output = pointer(args, 4)?;
                if let Some(used) = pointer(args, 7)? {
                    write_unsigned(memory, used, 4, i128::from(used_default))?;
                }
                if capacity != 0 {
                    if output.is_none() || capacity < bytes.len() as i64 {
                        set_last_error(memory, teb, 122)?;
                        return Ok(GuestCallResult::Value(GuestCallValue::Int32(0)));
                    }
                    memory.write(output.expect("output checked"), &bytes)?;
                }
                Ok(GuestCallResult::Value(GuestCallValue::Int32(bytes.len() as i32)))
            },
        ))?;
    }
    for wide in [false, true] {
        let name = if wide { "GetStringTypeW" } else { "GetStringTypeA" };
        let parameters: Vec<GuestStorage> = if wide {
            vec![GuestStorage::Uint32, GuestStorage::Pointer, GuestStorage::Int32, GuestStorage::Pointer]
        } else {
            vec![GuestStorage::Uint32, GuestStorage::Uint32, GuestStorage::Pointer, GuestStorage::Int32, GuestStorage::Pointer]
        };
        host.service("kernel32.dll", name, &parameters, Some(GuestStorage::Int32), Rc::new(
            move |ctx, context, args| {
                let _ = context;
                let shift = usize::from(!wide);
                if integer(args, shift)? != 1 {
                    return Err(unsupported_windows("kernel32.dll", "GetStringType", "only CT_CTYPE1 is implemented"));
                }
                let memory = ctx.memory();
                let source = string_input(memory, required_pointer(args, shift + 1)?, integer(args, shift + 2)? as i64, wide)?;
                let output = required_pointer(args, shift + 3)?;
                // Units, not scalar values: lone surrogates classify alone.
                let units: Vec<char> = source.encode_utf16().map(|unit| char::from_u32(unit as u32).unwrap_or(char::REPLACEMENT_CHARACTER)).collect();
                for (index, character) in units.iter().enumerate() {
                    let character = *character;
                    let code = character as u32;
                    let mut flags = 0u32;
                    if character.is_ascii_uppercase() {
                        flags |= 1;
                    }
                    if character.is_ascii_lowercase() {
                        flags |= 2;
                    }
                    if character.is_ascii_digit() {
                        flags |= 4;
                    }
                    if is_js_space(character) {
                        flags |= 8;
                    }
                    if matches!(code, 0x21..=0x2f | 0x3a..=0x40 | 0x5b..=0x60 | 0x7b..=0x7e) {
                        flags |= 16;
                    }
                    if code < 32 || code == 127 {
                        flags |= 32;
                    }
                    if character == ' ' || character == '\t' {
                        flags |= 64;
                    }
                    if character.is_ascii_hexdigit() {
                        flags |= 128;
                    }
                    if character.is_ascii_alphabetic() {
                        flags |= 256;
                    }
                    write_unsigned(memory, memory.offset(output, index as i64 * 2)?, 2, flags as i128)?;
                }
                Ok(GuestCallResult::Value(GuestCallValue::Int32(1)))
            },
        ))?;
    }
    for wide in [false, true] {
        let name = if wide { "LCMapStringW" } else { "LCMapStringA" };
        host.service("kernel32.dll", name, &[GuestStorage::Uint32, GuestStorage::Uint32, GuestStorage::Pointer, GuestStorage::Int32, GuestStorage::Pointer, GuestStorage::Int32], Some(GuestStorage::Int32), Rc::new(
            move |ctx, context, args| {
                let _ = context;
                let flags = integer(args, 1)? as u32;
                if flags != 0x100 && flags != 0x200 {
                    return Err(unsupported_windows("kernel32.dll", "LCMapString", format!("flags {flags}")));
                }
                let memory = ctx.memory();
                let source = string_input(memory, required_pointer(args, 2)?, integer(args, 3)? as i64, wide)?;
                let mapped: String = if flags == 0x100 {
                    source.chars().flat_map(|c| c.to_lowercase()).collect()
                } else {
                    source.chars().flat_map(|c| c.to_uppercase()).collect()
                };
                let utf16: Vec<u16> = mapped.encode_utf16().collect();
                let capacity = integer(args, 5)? as i64;
                let output = pointer(args, 4)?;
                if capacity != 0 {
                    if output.is_none() || capacity < utf16.len() as i64 {
                        set_last_error(memory, teb, 122)?;
                        return Ok(GuestCallResult::Value(GuestCallValue::Int32(0)));
                    }
                    let mut encoded = vec![0u8; utf16.len() * (if wide { 2 } else { 1 })];
                    if wide {
                        for (index, unit) in utf16.iter().enumerate() {
                            encoded[index * 2] = (unit & 0xff) as u8;
                            encoded[index * 2 + 1] = (unit >> 8) as u8;
                        }
                    } else {
                        for (index, unit) in utf16.iter().enumerate() {
                            encoded[index] = (unit & 0xff) as u8;
                        }
                    }
                    memory.write(output.expect("output checked"), &encoded)?;
                }
                Ok(GuestCallResult::Value(GuestCallValue::Int32(utf16.len() as i32)))
            },
        ))?;
    }
    Ok(())
}
