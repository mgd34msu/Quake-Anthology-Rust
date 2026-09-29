//! System V libc services: allocator, memory, strings, sorting, libm.
//!
//! Donor: `src/guest/runtime/system-v/libc.ts`.

use std::rc::Rc;

use crate::core::contracts::{GuestAccess, GuestAddress, GuestCallResult, GuestCallValue, GuestStorage};
use crate::error::GuestError;
use crate::runtime::common::memory::{
    allocate_bytes, argument, count, fill_bytes, integer, move_bytes, pointer, read_string,
    required_pointer, string_bytes, string_length,
};

fn floating(args: &[GuestCallValue], index: usize) -> Result<f64, GuestError> {
    match argument(args, index)? {
        GuestCallValue::Float32(value) => Ok(f64::from(*value)),
        GuestCallValue::Float64(value) => Ok(*value),
        _ => Err(GuestError::invalid("System V floating argument required")),
    }
}
use crate::runtime::system_v::contracts::{
    invoke_nested, tls_address, unsupported_system_v, SharedSystemV, SystemVServiceRegistrar,
};

fn size_value(pointer_bytes: usize, value: usize) -> GuestCallResult {
    if pointer_bytes == 4 {
        GuestCallResult::Value(GuestCallValue::Uint32(value as u32))
    } else {
        GuestCallResult::Value(GuestCallValue::Uint64(value as u64))
    }
}

fn signed_size_value(pointer_bytes: usize, value: i128) -> GuestCallResult {
    if pointer_bytes == 4 {
        GuestCallResult::Value(GuestCallValue::Int32((value as u32) as i32))
    } else {
        GuestCallResult::Value(GuestCallValue::Int64(value as i64))
    }
}

fn compare(left: &str, right: &str) -> i32 {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    let length = left.len().min(right.len());
    for index in 0..length {
        if left[index] != right[index] {
            return left[index] as i32 - right[index] as i32;
        }
    }
    if left.len() == right.len() {
        0
    } else if left.len() < right.len() {
        -(right[length] as i32)
    } else {
        left[length] as i32
    }
}

/// Register the libc service set.
pub fn install_libc(host: &mut SystemVServiceRegistrar<'_>) -> Result<(), GuestError> {
    crate::runtime::common::format::services::install_system_v_format(host)?;
    let pointer_bytes = host.pointer_bytes;
    let pointer_storage = host.pointer_storage();
    let signed_pointer_storage = host.signed_pointer_storage();
    let base = if pointer_bytes == 4 { "GLIBC_2.0" } else { "GLIBC_2.2.5" };
    let version: [Option<&str>; 2] = [Some(base), None];
    let lib = "libc.so.6";
    let errno = host.errno_address;
    let shared = Rc::clone(&host.shared);

    host.service(lib, "__errno_location", &version, &[], Some(GuestStorage::Pointer), Rc::new(
        move |_, _, _| Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(errno)))),
    ))?;

    if pointer_bytes == 8 {
        let shared = Rc::clone(&shared);
        let thread_pointer = host.thread_pointer;
        host.service("ld-linux-x86-64.so.2", "__tls_get_addr", &[Some("GLIBC_2.3"), None], &[GuestStorage::Pointer], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let memory = ctx.memory();
                let index = required_pointer(args, 0)?;
                let module_id = memory.read_u64(index)?;
                let offset = memory.read_u64(memory.offset(index, 8)?)?;
                let address = tls_address(&shared, memory, thread_pointer, module_id, offset)?;
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(address))))
            },
        ))?;
    }

    {
        let shared = Rc::clone(&shared);
        host.service(lib, "malloc", &version, &[pointer_storage], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let size = count(args, 0)?;
                let address = system_v_allocate(&shared, ctx.memory(), size)?;
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(address))))
            },
        ))?;
    }
    {
        let shared = Rc::clone(&shared);
        host.service(lib, "calloc", &version, &[pointer_storage, pointer_storage], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let total = integer(args, 0)? * integer(args, 1)?;
                if total > 0x1000_0000 {
                    ctx.memory().write_i32(errno, 12)?;
                    return Ok(GuestCallResult::Value(GuestCallValue::Pointer(None)));
                }
                let address = system_v_allocate(&shared, ctx.memory(), total as usize)?;
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(address))))
            },
        ))?;
    }
    {
        let shared = Rc::clone(&shared);
        host.service(lib, "free", &version, &[GuestStorage::Pointer], None, Rc::new(
            move |ctx, _, args| {
                system_v_free(&shared, ctx.memory(), pointer(args, 0)?)?;
                Ok(GuestCallResult::Void)
            },
        ))?;
    }
    {
        let shared = Rc::clone(&shared);
        host.service(lib, "realloc", &version, &[GuestStorage::Pointer, pointer_storage], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let old = pointer(args, 0)?;
                let size = count(args, 1)?;
                if size == 0 && old.is_some() {
                    system_v_free(&shared, ctx.memory(), old)?;
                    return Ok(GuestCallResult::Value(GuestCallValue::Pointer(None)));
                }
                let old_size = match old {
                    None => 0,
                    Some(address) => system_v_allocation_size(&shared, ctx.memory(), address)?.ok_or_else(|| {
                        GuestError::callback("System V realloc of non-live allocation")
                    })?,
                };
                let next = system_v_allocate(&shared, ctx.memory(), size)?;
                if let Some(old) = old {
                    let memory = ctx.memory();
                    move_bytes(memory, next, old, size.min(old_size))?;
                    system_v_free(&shared, memory, Some(old))?;
                }
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(next))))
            },
        ))?;
    }

    for name in ["memcpy", "memmove", "__memcpy_chk", "__memmove_chk"] {
        let checked = name.starts_with("__");
        let versions: Vec<Option<&str>> = if checked {
            vec![Some("GLIBC_2.3.4"), None]
        } else if name == "memcpy" && pointer_bytes == 8 {
            vec![Some(base), None, Some("GLIBC_2.14")]
        } else {
            vec![Some(base), None]
        };
        let name_owned = name.to_string();
        let parameters = if checked {
            vec![
                GuestStorage::Pointer,
                GuestStorage::Pointer,
                pointer_storage,
                pointer_storage,
            ]
        } else {
            vec![GuestStorage::Pointer, GuestStorage::Pointer, pointer_storage]
        };
        host.service(lib, name, &versions, &parameters, Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let destination = pointer(args, 0)?;
                let source = pointer(args, 1)?;
                let length = count(args, 2)?;
                if checked && (length as i128) > integer(args, 3)? {
                    return Err(GuestError::callback(format!(
                        "{name_owned} detected guest buffer overflow"
                    )));
                }
                if length != 0 {
                    let (Some(destination), Some(source)) = (destination, source) else {
                        return Err(GuestError::invalid("Nonnull guest memory arguments required"));
                    };
                    move_bytes(ctx.memory(), destination, source, length)?;
                }
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(destination)))
            },
        ))?;
    }

    host.service(lib, "memset", &version, &[GuestStorage::Pointer, GuestStorage::Int32, pointer_storage], Some(GuestStorage::Pointer), Rc::new(
        move |ctx, _, args| {
            let destination = pointer(args, 0)?;
            let length = count(args, 2)?;
            if length != 0 {
                let Some(destination_value) = destination else {
                    return Err(GuestError::invalid("Nonnull memset destination required"));
                };
                fill_bytes(ctx.memory(), destination_value, length, (integer(args, 1)? & 0xff) as u8)?;
            }
            Ok(GuestCallResult::Value(GuestCallValue::Pointer(destination)))
        },
    ))?;

    host.service(lib, "memcmp", &version, &[GuestStorage::Pointer, GuestStorage::Pointer, pointer_storage], Some(GuestStorage::Int32), Rc::new(
        move |ctx, _, args| {
            let length = count(args, 2)?;
            if length == 0 {
                return Ok(GuestCallResult::Value(GuestCallValue::Int32(0)));
            }
            let memory = ctx.memory();
            let left = memory.copy(required_pointer(args, 0)?, length)?;
            let right = memory.copy(required_pointer(args, 1)?, length)?;
            for index in 0..length {
                let delta = i32::from(left[index]) - i32::from(right[index]);
                if delta != 0 {
                    return Ok(GuestCallResult::Value(GuestCallValue::Int32(delta)));
                }
            }
            Ok(GuestCallResult::Value(GuestCallValue::Int32(0)))
        },
    ))?;

    host.service(lib, "strlen", &version, &[GuestStorage::Pointer], Some(pointer_storage), Rc::new(
        move |ctx, _, args| {
            let length = string_length(ctx.memory(), required_pointer(args, 0)?)?;
            Ok(size_value(pointer_bytes, length))
        },
    ))?;

    host.service(lib, "strcmp", &version, &[GuestStorage::Pointer, GuestStorage::Pointer], Some(GuestStorage::Int32), Rc::new(
        move |ctx, _, args| {
            let memory = ctx.memory();
            let left = read_string(memory, required_pointer(args, 0)?, false)?;
            let right = read_string(memory, required_pointer(args, 1)?, false)?;
            Ok(GuestCallResult::Value(GuestCallValue::Int32(compare(&left, &right))))
        },
    ))?;

    for name in ["strcpy", "stpcpy", "strcat", "__strcpy_chk", "__stpcpy_chk", "__strcat_chk"] {
        let checked = name.starts_with("__");
        let concatenate = name.contains("strcat");
        let end = name.contains("stpcpy");
        let versions: Vec<Option<&str>> = if checked {
            vec![Some("GLIBC_2.3.4"), None]
        } else {
            vec![Some(base), None]
        };
        let name_owned = name.to_string();
        let parameters = if checked {
            vec![GuestStorage::Pointer, GuestStorage::Pointer, pointer_storage]
        } else {
            vec![GuestStorage::Pointer, GuestStorage::Pointer]
        };
        host.service(lib, name, &versions, &parameters, Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let memory = ctx.memory();
                let destination = required_pointer(args, 0)?;
                let text = read_string(memory, required_pointer(args, 1)?, false)?;
                let prefix = if concatenate { string_length(memory, destination)? } else { 0 };
                if checked && (prefix + text.chars().count() + 1) as i128 > integer(args, 2)? {
                    return Err(GuestError::callback(format!(
                        "{name_owned} detected guest buffer overflow"
                    )));
                }
                memory.write(memory.offset(destination, prefix as i64)?, &string_bytes(&text, false))?;
                let result = if end {
                    memory.offset(destination, text.chars().count() as i64)?
                } else {
                    destination
                };
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(result))))
            },
        ))?;
    }

    host.service(lib, "strncpy", &version, &[GuestStorage::Pointer, GuestStorage::Pointer, pointer_storage], Some(GuestStorage::Pointer), Rc::new(
        move |ctx, _, args| {
            let length = count(args, 2)?;
            let destination = pointer(args, 0)?;
            if length == 0 {
                return Ok(GuestCallResult::Value(GuestCallValue::Pointer(destination)));
            }
            let Some(destination) = destination else {
                return Err(GuestError::invalid("Nonnull strncpy destination required"));
            };
            let memory = ctx.memory();
            let source = required_pointer(args, 1)?;
            let mut bytes = vec![0u8; length];
            for index in 0..length {
                let byte = memory.read_u8(memory.offset(source, index as i64)?)?;
                if byte == 0 {
                    break;
                }
                bytes[index] = byte;
            }
            memory.write(destination, &bytes)?;
            Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(destination))))
        },
    ))?;

    for name in ["strchr", "strrchr"] {
        let first = name == "strchr";
        host.service(lib, name, &version, &[GuestStorage::Pointer, GuestStorage::Int32], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let memory = ctx.memory();
                let source = required_pointer(args, 0)?;
                let text = format!("{}\0", read_string(memory, source, false)?);
                let needle = ((integer(args, 1)? & 0xff) as u8) as char;
                let chars: Vec<char> = text.chars().collect();
                let index = if first {
                    chars.iter().position(|c| *c == needle)
                } else {
                    chars.iter().rposition(|c| *c == needle)
                };
                let result = match index {
                    Some(index) => Some(memory.offset(source, index as i64)?),
                    None => None,
                };
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(result)))
            },
        ))?;
    }

    for name in ["strstr", "strpbrk"] {
        let substring = name == "strstr";
        host.service(lib, name, &version, &[GuestStorage::Pointer, GuestStorage::Pointer], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let memory = ctx.memory();
                let source = required_pointer(args, 0)?;
                let text = read_string(memory, source, false)?;
                let needle = read_string(memory, required_pointer(args, 1)?, false)?;
                // Offsets count UTF-16 units, like the donor's indexOf.
                let index = if substring {
                    text.find(&needle)
                        .map(|byte| text[..byte].chars().count())
                } else {
                    text.chars().position(|character| needle.contains(character))
                };
                let result = match index {
                    Some(index) => Some(memory.offset(source, index as i64)?),
                    None => None,
                };
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(result)))
            },
        ))?;
    }

    let token_slot = host.memory.allocate(&crate::core::contracts::GuestAllocationOptions {
        byte_length: pointer_bytes,
        alignment: 16,
        permissions: crate::core::contracts::GuestPermissions::ReadWrite,
        label: "System V strtok cursor".to_string(),
    })?;
    host.service(lib, "strtok", &version, &[GuestStorage::Pointer, GuestStorage::Pointer], Some(GuestStorage::Pointer), Rc::new(
        move |ctx, _, args| {
            let memory = ctx.memory();
            let stored = memory.read_pointer(token_slot)?;
            let source = pointer(args, 0)?.or(stored);
            let delimiters = read_string(memory, required_pointer(args, 1)?, false)?;
            let Some(source) = source else {
                return Ok(GuestCallResult::Value(GuestCallValue::Pointer(None)));
            };
            let text = read_string(memory, source, false)?;
            let chars: Vec<char> = text.chars().collect();
            let mut start = 0;
            while start < chars.len() && delimiters.contains(chars[start]) {
                start += 1;
            }
            if start == chars.len() {
                memory.write_pointer(token_slot, None)?;
                return Ok(GuestCallResult::Value(GuestCallValue::Pointer(None)));
            }
            let mut end = start;
            while end < chars.len() && !delimiters.contains(chars[end]) {
                end += 1;
            }
            if end < chars.len() {
                memory.write_u8(memory.offset(source, end as i64)?, 0)?;
                memory.write_pointer(token_slot, Some(memory.offset(source, (end + 1) as i64)?))?;
            } else {
                memory.write_pointer(token_slot, None)?;
            }
            Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(
                memory.offset(source, start as i64)?,
            ))))
        },
    ))?;

    {
        let shared = Rc::clone(&shared);
        let _ = &shared;
        host.service(lib, "strtol", &version, &[GuestStorage::Pointer, GuestStorage::Pointer, GuestStorage::Int32], Some(signed_pointer_storage), Rc::new(
            move |ctx, _, args| {
                let memory = ctx.memory();
                let source = required_pointer(args, 0)?;
                let end_pointer = pointer(args, 1)?;
                let text = read_string(memory, source, false)?;
                let chars: Vec<char> = text.chars().collect();
                let mut radix = integer(args, 2)?;
                let mut index = 0;
                let mut sign: i128 = 1;
                if radix != 0 && (radix < 2 || radix > 36) {
                    memory.write_i32(errno, 22)?;
                    if let Some(end_pointer) = end_pointer {
                        memory.write_pointer(end_pointer, Some(source))?;
                    }
                    return Ok(signed_size_value(pointer_bytes, 0));
                }
                while index < chars.len() && " \t\n\r\x0b\x0c".contains(chars[index]) {
                    index += 1;
                }
                if chars.get(index) == Some(&'-') || chars.get(index) == Some(&'+') {
                    if chars[index] == '-' {
                        sign = -1;
                    }
                    index += 1;
                }
                if (radix == 0 || radix == 16)
                    && index + 2 < chars.len() + 1
                    && chars.get(index) == Some(&'0')
                    && matches!(chars.get(index + 1), Some('x' | 'X'))
                    && chars
                        .get(index + 2)
                        .is_some_and(|c| c.is_ascii_hexdigit())
                {
                    radix = 16;
                    index += 2;
                }
                if radix == 0 {
                    radix = if chars.get(index) == Some(&'0') { 8 } else { 10 };
                }
                let first = index;
                let limit: i128 = 1 << (pointer_bytes * 8 - 1);
                let mut value: i128 = 0;
                while index < chars.len() {
                    let digit = "0123456789abcdefghijklmnopqrstuvwxyz"
                        .find(chars[index].to_ascii_lowercase());
                    let Some(digit) = digit else { break };
                    if digit as i128 >= radix {
                        break;
                    }
                    if value <= limit {
                        value = value * radix + digit as i128;
                    }
                    index += 1;
                }
                if let Some(end_pointer) = end_pointer {
                    memory.write_pointer(
                        end_pointer,
                        Some(memory.offset(source, (if index == first { 0 } else { index }) as i64)?),
                    )?;
                }
                value *= sign;
                if value < -limit || value >= limit {
                    memory.write_i32(errno, 34)?;
                    value = if value < 0 { -limit } else { limit - 1 };
                }
                Ok(signed_size_value(pointer_bytes, value))
            },
        ))?;
    }

    {
        let now_seconds = host.shared.borrow().capabilities.now_seconds.clone();
        let base_owned = base.to_string();
        host.service(lib, "time", &version, &[GuestStorage::Pointer], Some(signed_pointer_storage), Rc::new(
            move |ctx, context, args| {
                let Some(now_seconds) = &now_seconds else {
                    return Err(unsupported_system_v(
                        lib,
                        "time",
                        Some(&base_owned),
                        "no deterministic clock capability supplied",
                    ));
                };
                let _ = context;
                let value = now_seconds();
                let memory = ctx.memory();
                if let Some(destination) = pointer(args, 0)? {
                    if pointer_bytes == 4 {
                        memory.write_i32(destination, (value as u32) as i32)?;
                    } else {
                        memory.write_i64(destination, value)?;
                    }
                }
                Ok(signed_size_value(pointer_bytes, i128::from(value)))
            },
        ))?;
    }

    {
        let shared = Rc::clone(&shared);
        host.service(lib, "qsort", &version, &[GuestStorage::Pointer, pointer_storage, pointer_storage, GuestStorage::Pointer], None, Rc::new(
            move |ctx, context, args| {
                let base_address = pointer(args, 0)?;
                let length = count(args, 1)?;
                let size = count(args, 2)?;
                let comparator = required_pointer(args, 3)?;
                if length < 2 || size == 0 {
                    return Ok(GuestCallResult::Void);
                }
                let Some(base_address) = base_address else {
                    return Err(GuestError::invalid("qsort range outside supported guest memory"));
                };
                if length.saturating_mul(size) > 0x1000_0000 {
                    return Err(GuestError::invalid("qsort range outside supported guest memory"));
                }
                ctx.memory().check(base_address, length * size, GuestAccess::Write)?;
                qsort_heapsort(ctx, context, &shared, pointer_bytes, base_address, length, size, comparator)?;
                Ok(GuestCallResult::Void)
            },
        ))?;
    }

    install_libm(host, lib, &version)?;

    Ok(())
}

fn install_libm(
    host: &mut SystemVServiceRegistrar<'_>,
    lib: &str,
    version: &[Option<&str>],
) -> Result<(), GuestError> {
    for name in ["sin", "cos", "ceil", "floor", "sqrt", "fabs"] {
        for float32 in [true, false] {
            let storage = if float32 { GuestStorage::Float32 } else { GuestStorage::Float64 };
            let symbol = if float32 { format!("{name}f") } else { name.to_string() };
            host.service("libm.so.6", &symbol, version, &[storage], Some(storage), Rc::new(
                move |_, _, args| {
                    let value = floating(args, 0)?;
                    let result = match name {
                        "sin" => value.sin(),
                        "cos" => value.cos(),
                        "ceil" => value.ceil(),
                        "floor" => value.floor(),
                        "sqrt" => value.sqrt(),
                        _ => value.abs(),
                    };
                    Ok(if float32 {
                        GuestCallResult::Value(GuestCallValue::Float32(result as f32))
                    } else {
                        GuestCallResult::Value(GuestCallValue::Float64(result))
                    })
                },
            ))?;
        }
    }
    host.service("libm.so.6", "__atan2_finite", &[Some("GLIBC_2.15"), None], &[GuestStorage::Float64, GuestStorage::Float64], Some(GuestStorage::Float64), Rc::new(
        move |_, _, args| {
            Ok(GuestCallResult::Value(GuestCallValue::Float64(
                floating(args, 0)?.atan2(floating(args, 1)?),
            )))
        },
    ))?;
    let pointer_bytes = host.pointer_bytes;
    for float32 in [true, false] {
        let storage = if float32 { GuestStorage::Float32 } else { GuestStorage::Float64 };
        let symbol = if float32 { "sincosf" } else { "sincos" };
        let version_name = if pointer_bytes == 4 { "GLIBC_2.1" } else { "GLIBC_2.2.5" };
        host.service("libm.so.6", symbol, &[Some(version_name), None], &[storage, GuestStorage::Pointer, GuestStorage::Pointer], None, Rc::new(
            move |ctx, _, args| {
                let value = floating(args, 0)?;
                let memory = ctx.memory();
                let sine = required_pointer(args, 1)?;
                let cosine = required_pointer(args, 2)?;
                if float32 {
                    memory.write_f32(sine, value.sin() as f32)?;
                    memory.write_f32(cosine, value.cos() as f32)?;
                } else {
                    memory.write_f64(sine, value.sin())?;
                    memory.write_f64(cosine, value.cos())?;
                }
                Ok(GuestCallResult::Void)
            },
        ))?;
    }
    host.service(lib, "__isnanf", version, &[GuestStorage::Float32], Some(GuestStorage::Int32), Rc::new(
        move |_, _, args| {
            Ok(GuestCallResult::Value(GuestCallValue::Int32(i32::from(
                floating(args, 0)?.is_nan(),
            ))))
        },
    ))?;
    let stack_lib = lib.to_string();
    host.service(lib, "__stack_chk_fail", &[Some("GLIBC_2.4"), None], &[], None, Rc::new(
        move |_, context, _| {
            let _ = context;
            Err(unsupported_system_v(
                &stack_lib,
                "__stack_chk_fail",
                Some("GLIBC_2.4"),
                "guest stack protection failure",
            ))
        },
    ))?;
    Ok(())
}

/// Heap sort over guest elements with a guest comparator.
#[allow(clippy::too_many_arguments)]
fn qsort_heapsort(
    ctx: &mut crate::core::callbacks::HostCallContext<'_, '_>,
    context: &crate::core::contracts::GuestCallContext,
    shared: &SharedSystemV,
    pointer_bytes: usize,
    base_address: crate::core::contracts::GuestAddress,
    length: usize,
    size: usize,
    comparator: crate::core::contracts::GuestAddress,
) -> Result<(), GuestError> {
    let at = |ctx: &mut crate::core::callbacks::HostCallContext<'_, '_>, index: usize| -> Result<crate::core::contracts::GuestAddress, GuestError> {
        ctx.memory().offset(base_address, (index * size) as i64)
    };
    let less = |ctx: &mut crate::core::callbacks::HostCallContext<'_, '_>, a: usize, b: usize| -> Result<bool, GuestError> {
        let aa = at(ctx, a)?;
        let bb = at(ctx, b)?;
        let result = invoke_nested(
            ctx,
            shared,
            pointer_bytes,
            context,
            comparator,
            &[GuestStorage::Pointer, GuestStorage::Pointer],
            Some(GuestStorage::Int32),
            vec![
                GuestCallValue::Pointer(Some(aa)),
                GuestCallValue::Pointer(Some(bb)),
            ],
        )?;
        match result {
            GuestCallResult::Value(GuestCallValue::Int32(value)) => Ok(value < 0),
            _ => Err(GuestError::invalid("qsort comparator returned wrong type")),
        }
    };
    let swap = |ctx: &mut crate::core::callbacks::HostCallContext<'_, '_>, a: usize, b: usize| -> Result<(), GuestError> {
        let memory = ctx.memory();
        let aa = memory.offset(base_address, (a * size) as i64)?;
        let bb = memory.offset(base_address, (b * size) as i64)?;
        let bytes = memory.copy(aa, size)?;
        let other = memory.copy(bb, size)?;
        memory.write(aa, &other)?;
        memory.write(bb, &bytes)?;
        Ok(())
    };
    let sift = |ctx: &mut crate::core::callbacks::HostCallContext<'_, '_>, root: usize, end: usize| -> Result<(), GuestError> {
        let mut root = root;
        loop {
            let mut child = root * 2 + 1;
            if child >= end {
                break;
            }
            if child + 1 < end && less(ctx, child, child + 1)? {
                child += 1;
            }
            if !less(ctx, root, child)? {
                break;
            }
            swap(ctx, root, child)?;
            root = child;
        }
        Ok(())
    };
    for index in (0..length / 2).rev() {
        sift(ctx, index, length)?;
    }
    for end in (1..length).rev() {
        swap(ctx, 0, end)?;
        sift(ctx, 0, end)?;
    }
    Ok(())
}

/// Allocate heap memory from a host closure.
pub fn system_v_allocate(
    shared: &SharedSystemV,
    memory: &mut crate::core::memory::SparseGuestMemory,
    size: usize,
) -> Result<GuestAddress, GuestError> {
    if size > 0x1000_0000 {
        return Err(GuestError::invalid(
            "System V guest allocation exceeds supported size",
        ));
    }
    let address = allocate_bytes(memory, size.max(1), "System V guest heap")?;
    shared.borrow_mut().heap.insert(
        address.offset,
        crate::runtime::system_v::contracts::SystemVHeapEntry {
            address,
            size: size.max(1),
        },
    );
    Ok(address)
}

/// Release heap memory from a host closure.
pub fn system_v_free(
    shared: &SharedSystemV,
    memory: &mut crate::core::memory::SparseGuestMemory,
    address: Option<GuestAddress>,
) -> Result<(), GuestError> {
    let Some(address) = address else {
        return Ok(());
    };
    memory.offset(address, 0)?;
    let entry = shared.borrow_mut().heap.remove(&address.offset);
    let Some(entry) = entry else {
        return Err(GuestError::callback("System V free of a non-live allocation"));
    };
    memory.unmap(entry.address, entry.size)
}

/// Live allocation size from a host closure.
pub fn system_v_allocation_size(
    shared: &SharedSystemV,
    memory: &mut crate::core::memory::SparseGuestMemory,
    address: GuestAddress,
) -> Result<Option<usize>, GuestError> {
    memory.offset(address, 0)?;
    Ok(shared.borrow().heap.get(&address.offset).map(|entry| entry.size))
}
