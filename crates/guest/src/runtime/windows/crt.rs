//! Windows CRT services: heap, strings, onexit tables, math, conversions.
//!
//! Donor: `src/guest/runtime/windows/crt.ts`.

use std::rc::Rc;

use crate::core::callbacks::{HostCallContext, HostCallbackFn};
use crate::core::contracts::{
    GuestAccess, GuestAddress, GuestCallContext, GuestCallResult, GuestCallValue, GuestStorage,
};
use crate::core::memory::SparseGuestMemory;
use crate::error::GuestError;
use crate::runtime::common::memory::{
    count, fill_bytes, integer, move_bytes, pointer, read_string, required_pointer, string_length,
};
use crate::runtime::windows::contracts::{
    invoke_nested, now_millis, unsupported_windows, SharedWindows, WindowsContext, WindowsServiceRegistrar,
};
use crate::runtime::windows::time::{local_offset_at, tm_fields};

fn service(
    host: &mut WindowsServiceRegistrar<'_>,
    family: &str,
    name: &str,
    parameters: &[GuestStorage],
    result: Option<GuestStorage>,
    invoke: HostCallbackFn,
) -> Result<(), GuestError> {
    let mut libraries = Vec::new();
    if family != "vcruntime" {
        libraries.push(format!("api-ms-win-crt-{family}-l1-1-0.dll"));
    }
    libraries.push("ucrtbase.dll".to_string());
    libraries.push("msvcrt.dll".to_string());
    for library in &libraries {
        host.service(library, name, parameters, result, Rc::clone(&invoke))?;
    }
    Ok(())
}

fn u32_value(value: u32) -> GuestCallResult {
    GuestCallResult::Value(GuestCallValue::Uint32(value))
}

fn i32_value(value: i32) -> GuestCallResult {
    GuestCallResult::Value(GuestCallValue::Int32(value))
}

fn ptr_value(value: Option<GuestAddress>) -> GuestCallResult {
    GuestCallResult::Value(GuestCallValue::Pointer(value))
}

fn zero() -> GuestCallResult {
    i32_value(0)
}

fn done() -> GuestCallResult {
    GuestCallResult::Void
}

/// Register the CRT service set.
pub fn install_crt(host: &mut WindowsServiceRegistrar<'_>) -> Result<(), GuestError> {
    let width = host.pointer_bytes;
    let pointer_storage = host.pointer_storage();
    let teb = host.teb;
    let context = host.context.clone();
    {
        let shared = Rc::clone(&host.shared);
        host.service(
            "msvcp140.dll",
            "_Xtime_get_ticks",
            &[],
            Some(GuestStorage::Int64),
            Rc::new(move |_, _, _| {
                // Microsoft STL xtime.cpp: FILETIME ticks, but the runtime's own
                // clock instead of the host wall clock.
                Ok(GuestCallResult::Value(GuestCallValue::Int64(
                    now_millis(&shared).saturating_mul(10_000),
                )))
            }),
        )?;
    }
    {
        let context = context.clone();
        service(
            host,
            "heap",
            "malloc",
            &[pointer_storage],
            Some(GuestStorage::Pointer),
            Rc::new(move |ctx, _, args| {
                let memory = ctx.memory();
                Ok(ptr_value(context.allocate(memory, teb, count(args, 0)?, 0)?))
            }),
        )?;
    }
    {
        let context = context.clone();
        service(
            host,
            "heap",
            "calloc",
            &[pointer_storage, pointer_storage],
            Some(GuestStorage::Pointer),
            Rc::new(move |ctx, _, args| {
                let size = integer(args, 0)? * integer(args, 1)?;
                if size > 0x1000_0000 {
                    return Ok(ptr_value(None));
                }
                let memory = ctx.memory();
                Ok(ptr_value(context.allocate(memory, teb, size as usize, 0)?))
            }),
        )?;
    }
    {
        let context = context.clone();
        service(
            host,
            "heap",
            "free",
            &[GuestStorage::Pointer],
            None,
            Rc::new(move |ctx, _, args| {
                let memory = ctx.memory();
                if !context.free(memory, teb, pointer(args, 0)?, 0)? {
                    return Err(GuestError::callback("CRT free of invalid guest allocation"));
                }
                Ok(done())
            }),
        )?;
    }
    service(
        host,
        "heap",
        "_callnewh",
        &[pointer_storage],
        Some(GuestStorage::Int32),
        Rc::new(move |_, _, _| Ok(zero())),
    )?;
    for name in ["memcpy", "memmove"] {
        service(
            host,
            "vcruntime",
            name,
            &[GuestStorage::Pointer, GuestStorage::Pointer, pointer_storage],
            Some(GuestStorage::Pointer),
            Rc::new(move |ctx, _, args| {
                let destination = pointer(args, 0)?;
                if count(args, 2)? > 0 {
                    move_bytes(
                        ctx.memory(),
                        required_pointer(args, 0)?,
                        required_pointer(args, 1)?,
                        count(args, 2)?,
                    )?;
                }
                Ok(ptr_value(destination))
            }),
        )?;
    }
    service(
        host,
        "vcruntime",
        "memset",
        &[GuestStorage::Pointer, GuestStorage::Int32, pointer_storage],
        Some(GuestStorage::Pointer),
        Rc::new(move |ctx, _, args| {
            let destination = required_pointer(args, 0)?;
            let value = (integer(args, 1)? & 255) as u8;
            fill_bytes(ctx.memory(), destination, count(args, 2)?, value)?;
            Ok(ptr_value(Some(destination)))
        }),
    )?;
    service(
        host,
        "vcruntime",
        "memcmp",
        &[GuestStorage::Pointer, GuestStorage::Pointer, pointer_storage],
        Some(GuestStorage::Int32),
        Rc::new(move |ctx, _, args| {
            let memory = ctx.memory();
            let left = memory.copy(required_pointer(args, 0)?, count(args, 2)?)?;
            let right = memory.copy(required_pointer(args, 1)?, left.len())?;
            for index in 0..left.len() {
                let delta = i32::from(left[index]) - i32::from(right[index]);
                if delta != 0 {
                    return Ok(i32_value(delta));
                }
            }
            Ok(zero())
        }),
    )?;
    service(
        host,
        "string",
        "memchr",
        &[GuestStorage::Pointer, GuestStorage::Int32, pointer_storage],
        Some(GuestStorage::Pointer),
        Rc::new(move |ctx, _, args| {
            let memory = ctx.memory();
            let source = required_pointer(args, 0)?;
            let size = count(args, 2)?;
            if size == 0 {
                return Ok(ptr_value(None));
            }
            let bytes = memory.copy(source, size)?;
            let needle = (integer(args, 1)? & 255) as u8;
            let result = match bytes.iter().position(|byte| *byte == needle) {
                Some(index) => Some(memory.offset(source, index as i64)?),
                None => None,
            };
            Ok(ptr_value(result))
        }),
    )?;
    service(
        host,
        "string",
        "strlen",
        &[GuestStorage::Pointer],
        Some(pointer_storage),
        Rc::new(move |ctx, _, args| {
            let length = string_length(ctx.memory(), required_pointer(args, 0)?)?;
            Ok(if width == 4 {
                u32_value(length as u32)
            } else {
                GuestCallResult::Value(GuestCallValue::Uint64(length as u64))
            })
        }),
    )?;
    for (name, bounded) in [("strcmp", false), ("strncmp", true)] {
        let parameters: Vec<GuestStorage> = if bounded {
            vec![GuestStorage::Pointer, GuestStorage::Pointer, pointer_storage]
        } else {
            vec![GuestStorage::Pointer, GuestStorage::Pointer]
        };
        service(
            host,
            "string",
            name,
            &parameters,
            Some(GuestStorage::Int32),
            Rc::new(move |ctx, _, args| {
                let maximum = if bounded { count(args, 2)? } else { 1024 * 1024 };
                let memory = ctx.memory();
                let left = required_pointer(args, 0)?;
                let right = required_pointer(args, 1)?;
                for index in 0..maximum {
                    let a = memory.read_u8(memory.offset(left, index as i64)?)?;
                    let b = memory.read_u8(memory.offset(right, index as i64)?)?;
                    if a != b {
                        return Ok(i32_value(i32::from(a) - i32::from(b)));
                    }
                    if a == 0 {
                        return Ok(zero());
                    }
                }
                Ok(zero())
            }),
        )?;
    }
    service(
        host,
        "string",
        "strchr",
        &[GuestStorage::Pointer, GuestStorage::Int32],
        Some(GuestStorage::Pointer),
        Rc::new(move |ctx, _, args| {
            let memory = ctx.memory();
            let source = required_pointer(args, 0)?;
            let text = format!("{}\0", read_string(memory, source, false)?);
            let needle = ((integer(args, 1)? & 255) as u8) as char;
            let result = match text.chars().position(|c| c == needle) {
                Some(index) => Some(memory.offset(source, index as i64)?),
                None => None,
            };
            Ok(ptr_value(result))
        }),
    )?;
    service(
        host,
        "string",
        "strstr",
        &[GuestStorage::Pointer, GuestStorage::Pointer],
        Some(GuestStorage::Pointer),
        Rc::new(move |ctx, _, args| {
            let memory = ctx.memory();
            let source = required_pointer(args, 0)?;
            let text = read_string(memory, source, false)?;
            let needle = read_string(memory, required_pointer(args, 1)?, false)?;
            let result = match text.find(&needle).map(|byte| text[..byte].chars().count()) {
                Some(index) => Some(memory.offset(source, index as i64)?),
                None => None,
            };
            Ok(ptr_value(result))
        }),
    )?;
    service(
        host,
        "startup",
        "_configure_narrow_argv",
        &[GuestStorage::Int32],
        Some(GuestStorage::Int32),
        Rc::new(move |_, context, args| {
            let _ = context;
            if !(0..=2).contains(&integer(args, 0)?) {
                return Err(unsupported_windows(
                    "ucrtbase.dll",
                    "_configure_narrow_argv",
                    "invalid argument mode",
                ));
            }
            Ok(zero())
        }),
    )?;
    service(
        host,
        "startup",
        "_initialize_narrow_environment",
        &[],
        Some(GuestStorage::Int32),
        Rc::new(move |_, _, _| Ok(zero())),
    )?;
    install_onexit(host, width, teb, &context)?;
    {
        let shared = Rc::clone(&host.shared);
        service(
            host,
            "utility",
            "qsort",
            &[
                GuestStorage::Pointer,
                pointer_storage,
                pointer_storage,
                GuestStorage::Pointer,
            ],
            None,
            Rc::new(move |ctx, context, args| {
                let length = count(args, 1)?;
                if length < 2 {
                    return Ok(done());
                }
                let size = count(args, 2)?;
                if size == 0 || length.checked_mul(size).is_none_or(|total| total > 0x1000_0000) {
                    return Err(GuestError::invalid("Invalid guest qsort extent"));
                }
                let base = required_pointer(args, 0)?;
                let comparator = required_pointer(args, 3)?;
                ctx.memory().check(base, length * size, GuestAccess::Write)?;
                qsort_heapsort(ctx, context, &shared, width, base, length, size, comparator)?;
                Ok(done())
            }),
        )?;
    }
    install_math(host)?;
    install_conversions(host)?;
    Ok(())
}

fn install_onexit(
    host: &mut WindowsServiceRegistrar<'_>,
    width: usize,
    teb: GuestAddress,
    context: &WindowsContext,
) -> Result<(), GuestError> {
    let onexit = context
        .allocate(host.memory, teb, width * 3, 0)?
        .ok_or_else(|| GuestError::callback("Windows onexit table allocation failed"))?;
    let shared = Rc::clone(&host.shared);
    service(
        host,
        "runtime",
        "_initialize_onexit_table",
        &[GuestStorage::Pointer],
        Some(GuestStorage::Int32),
        Rc::new(move |ctx, _, args| {
            ctx.memory().write(required_pointer(args, 0)?, &vec![0u8; width * 3])?;
            Ok(zero())
        }),
    )?;
    {
        let context = context.clone();
        service(
            host,
            "runtime",
            "_register_onexit_function",
            &[GuestStorage::Pointer, GuestStorage::Pointer],
            Some(GuestStorage::Int32),
            Rc::new(move |ctx, _, args| {
                register_exit(
                    ctx.memory(),
                    &context,
                    teb,
                    width,
                    required_pointer(args, 0)?,
                    required_pointer(args, 1)?,
                )
            }),
        )?;
    }
    {
        let context = context.clone();
        service(
            host,
            "runtime",
            "_crt_atexit",
            &[GuestStorage::Pointer],
            Some(GuestStorage::Int32),
            Rc::new(move |ctx, _, args| {
                register_exit(ctx.memory(), &context, teb, width, onexit, required_pointer(args, 0)?)
            }),
        )?;
    }
    {
        let shared = Rc::clone(&shared);
        let runtime = context.clone();
        service(
            host,
            "runtime",
            "_execute_onexit_table",
            &[GuestStorage::Pointer],
            Some(GuestStorage::Int32),
            Rc::new(move |ctx, context, args| {
                execute_exit(ctx, context, &shared, &runtime, teb, width, required_pointer(args, 0)?)?;
                Ok(zero())
            }),
        )?;
    }
    {
        let shared = Rc::clone(&shared);
        let runtime = context.clone();
        service(
            host,
            "runtime",
            "_cexit",
            &[],
            None,
            Rc::new(move |ctx, context, _| {
                execute_exit(ctx, context, &shared, &runtime, teb, width, onexit)?;
                Ok(done())
            }),
        )?;
    }
    for (name, result) in [("_initterm", None), ("_initterm_e", Some(GuestStorage::Int32))] {
        let shared = Rc::clone(&shared);
        service(
            host,
            "runtime",
            name,
            &[GuestStorage::Pointer, GuestStorage::Pointer],
            result,
            Rc::new(move |ctx, context, args| {
                let begin = required_pointer(args, 0)?;
                let end = required_pointer(args, 1)?;
                if end.offset < begin.offset
                    || (end.offset - begin.offset) % width as u64 != 0
                    || end.offset - begin.offset > 1024 * 1024
                {
                    return Err(GuestError::callback("Invalid CRT initializer range"));
                }
                let mut at = begin.offset;
                while at < end.offset {
                    let slot = ctx
                        .memory()
                        .pointer(at)?
                        .ok_or_else(|| GuestError::callback("Null CRT initializer slot"))?;
                    at += width as u64;
                    let callback = ctx.memory().read_pointer(slot)?;
                    let Some(callback) = callback else {
                        continue;
                    };
                    let outcome = invoke_nested(ctx, &shared, width, context, callback, &[], result, vec![], false)?;
                    if result.is_some() {
                        let GuestCallResult::Value(GuestCallValue::Int32(value)) = outcome else {
                            return Err(GuestError::invalid("CRT initializer returned wrong type"));
                        };
                        if value != 0 {
                            return Ok(i32_value(value));
                        }
                    }
                }
                Ok(if result.is_some() { zero() } else { done() })
            }),
        )?;
    }
    host.service(
        "vcruntime140.dll",
        "__std_type_info_destroy_list",
        &[GuestStorage::Pointer],
        None,
        Rc::new(move |ctx, context, args| {
            let _ = context;
            let memory = ctx.memory();
            let bytes = memory.copy(required_pointer(args, 0)?, if width == 4 { 8 } else { 16 })?;
            if bytes.iter().any(|byte| *byte != 0) {
                return Err(unsupported_windows(
                    "vcruntime140.dll",
                    "__std_type_info_destroy_list",
                    "populated type-name SLIST destruction is not implemented",
                ));
            }
            Ok(done())
        }),
    )?;
    Ok(())
}

fn register_exit(
    memory: &mut SparseGuestMemory,
    context: &WindowsContext,
    teb: GuestAddress,
    width: usize,
    table: GuestAddress,
    callback: GuestAddress,
) -> Result<GuestCallResult, GuestError> {
    let begin = memory.read_pointer(table)?;
    let mut end = memory.read_pointer(memory.offset(table, width as i64)?)?;
    let capacity = memory.read_pointer(memory.offset(table, 2 * width as i64)?)?;
    if begin.is_none() || end.is_none() || capacity.is_none() || end == capacity {
        let existing = match (begin, end) {
            (Some(begin), Some(end)) => (end.offset - begin.offset) as usize,
            _ => 0,
        };
        let next = context.allocate(memory, teb, (32 * width).max(existing * 2), 0)?;
        let Some(next) = next else {
            return Ok(i32_value(-1));
        };
        if let (Some(begin), Some(end)) = (begin, end) {
            let bytes = memory.copy(begin, (end.offset - begin.offset) as usize)?;
            memory.write(next, &bytes)?;
            context.free(memory, teb, Some(begin), 0)?;
        }
        // Reserve one slot per onexit function: the scan runs before this
        // registration, so the used-end stays behind the new capacity.
        let begin = next;
        end = Some(memory.offset(next, existing as i64)?);
        let capacity = memory.offset(next, (32 * width).max(existing * 2) as i64)?;
        memory.write_pointer(table, Some(begin))?;
        memory.write_pointer(memory.offset(table, 2 * width as i64)?, Some(capacity))?;
    }
    let end = end.expect("onexit end checked");
    memory.write_pointer(end, Some(callback))?;
    memory.write_pointer(
        memory.offset(table, width as i64)?,
        Some(memory.offset(end, width as i64)?),
    )?;
    Ok(zero())
}

fn execute_exit(
    ctx: &mut HostCallContext<'_, '_>,
    context: &GuestCallContext,
    shared: &SharedWindows,
    runtime: &WindowsContext,
    teb: GuestAddress,
    width: usize,
    table: GuestAddress,
) -> Result<(), GuestError> {
    let memory = ctx.memory();
    let begin = memory.read_pointer(table)?;
    let end = memory.read_pointer(memory.offset(table, width as i64)?)?;
    if let (Some(begin), Some(end)) = (begin, end) {
        let mut at = end.offset;
        while at > begin.offset {
            at -= width as u64;
            let callback = {
                let memory = ctx.memory();
                let slot = memory
                    .pointer(at)?
                    .ok_or_else(|| GuestError::callback("Null onexit slot"))?;
                let callback = memory.read_pointer(slot)?;
                memory.write_pointer(slot, None)?;
                callback
            };
            if let Some(callback) = callback {
                invoke_nested(ctx, shared, width, context, callback, &[], None, vec![], false)?;
            }
        }
        runtime.free(ctx.memory(), teb, Some(begin), 0)?;
    }
    ctx.memory().write(table, &vec![0u8; width * 3])?;
    Ok(())
}

/// Heap sort over guest elements with a guest comparator.
#[allow(clippy::too_many_arguments)]
fn qsort_heapsort(
    ctx: &mut HostCallContext<'_, '_>,
    context: &GuestCallContext,
    shared: &SharedWindows,
    width: usize,
    base: GuestAddress,
    length: usize,
    size: usize,
    comparator: GuestAddress,
) -> Result<(), GuestError> {
    let at = |ctx: &mut HostCallContext<'_, '_>, index: usize| -> Result<GuestAddress, GuestError> {
        ctx.memory().offset(base, (index * size) as i64)
    };
    let compare = |ctx: &mut HostCallContext<'_, '_>, a: usize, b: usize| -> Result<i32, GuestError> {
        let aa = at(ctx, a)?;
        let bb = at(ctx, b)?;
        let result = invoke_nested(
            ctx,
            shared,
            width,
            context,
            comparator,
            &[GuestStorage::Pointer, GuestStorage::Pointer],
            Some(GuestStorage::Int32),
            vec![GuestCallValue::Pointer(Some(aa)), GuestCallValue::Pointer(Some(bb))],
            false,
        )?;
        match result {
            GuestCallResult::Value(GuestCallValue::Int32(value)) => Ok(value),
            _ => Err(GuestError::invalid("qsort comparator returned wrong type")),
        }
    };
    let swap = |ctx: &mut HostCallContext<'_, '_>, a: usize, b: usize| -> Result<(), GuestError> {
        let memory = ctx.memory();
        let aa = memory.offset(base, (a * size) as i64)?;
        let bb = memory.offset(base, (b * size) as i64)?;
        let bytes = memory.copy(aa, size)?;
        let other = memory.copy(bb, size)?;
        memory.write(aa, &other)?;
        memory.write(bb, &bytes)?;
        Ok(())
    };
    let sift = |ctx: &mut HostCallContext<'_, '_>, root: usize, end: usize| -> Result<(), GuestError> {
        let mut root = root;
        let mut child = root * 2 + 1;
        while child < end {
            if child + 1 < end && compare(ctx, child, child + 1)? < 0 {
                child += 1;
            }
            if compare(ctx, root, child)? >= 0 {
                return Ok(());
            }
            swap(ctx, root, child)?;
            root = child;
            child = root * 2 + 1;
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

fn real(args: &[GuestCallValue], index: usize) -> Result<f64, GuestError> {
    match crate::runtime::common::memory::argument(args, index)? {
        GuestCallValue::Float32(value) => Ok(f64::from(*value)),
        GuestCallValue::Float64(value) => Ok(*value),
        _ => Err(GuestError::invalid("CRT floating argument required")),
    }
}

fn install_math(host: &mut WindowsServiceRegistrar<'_>) -> Result<(), GuestError> {
    for name in [
        "acosf", "sinf", "ceilf", "cosf", "truncf", "log2f", "floorf", "sqrtf", "tanf",
    ] {
        service(
            host,
            "math",
            name,
            &[GuestStorage::Float32],
            Some(GuestStorage::Float32),
            Rc::new(move |_, _, args| {
                let value = real(args, 0)?;
                let result = match name {
                    "acosf" => value.acos(),
                    "sinf" => value.sin(),
                    "ceilf" => value.ceil(),
                    "cosf" => value.cos(),
                    "truncf" => value.trunc(),
                    "log2f" => value.log2(),
                    "floorf" => value.floor(),
                    "sqrtf" => value.sqrt(),
                    _ => value.tan(),
                };
                Ok(GuestCallResult::Value(GuestCallValue::Float32(result as f32)))
            }),
        )?;
    }
    service(
        host,
        "math",
        "atan2f",
        &[GuestStorage::Float32, GuestStorage::Float32],
        Some(GuestStorage::Float32),
        Rc::new(move |_, _, args| {
            Ok(GuestCallResult::Value(GuestCallValue::Float32(
                real(args, 0)?.atan2(real(args, 1)?) as f32,
            )))
        }),
    )?;
    service(
        host,
        "math",
        "fmodf",
        &[GuestStorage::Float32, GuestStorage::Float32],
        Some(GuestStorage::Float32),
        Rc::new(move |_, _, args| {
            Ok(GuestCallResult::Value(GuestCallValue::Float32(
                (real(args, 0)? % real(args, 1)?) as f32,
            )))
        }),
    )?;
    service(
        host,
        "math",
        "pow",
        &[GuestStorage::Float64, GuestStorage::Float64],
        Some(GuestStorage::Float64),
        Rc::new(move |_, _, args| {
            Ok(GuestCallResult::Value(GuestCallValue::Float64(
                real(args, 0)?.powf(real(args, 1)?),
            )))
        }),
    )?;
    service(
        host,
        "math",
        "nextafterf",
        &[GuestStorage::Float32, GuestStorage::Float32],
        Some(GuestStorage::Float32),
        Rc::new(move |_, _, args| {
            let from = real(args, 0)? as f32;
            let to = real(args, 1)? as f32;
            let result = if from.is_nan() || to.is_nan() {
                f32::NAN
            } else if from == to {
                to
            } else {
                let bits = from.to_bits();
                let magnitude = bits & 0x7fff_ffff;
                let direction = (to > from) != (from < 0.0);
                let next = if direction {
                    if magnitude == 0x7f80_0000 {
                        return Ok(GuestCallResult::Value(GuestCallValue::Float32(if from < 0.0 {
                            f32::NEG_INFINITY
                        } else {
                            f32::INFINITY
                        })));
                    }
                    bits + 1
                } else if magnitude == 0 {
                    0x8000_0000 + 1 - bits
                } else {
                    bits - 1
                };
                f32::from_bits(next)
            };
            Ok(GuestCallResult::Value(GuestCallValue::Float32(result)))
        }),
    )?;
    service(
        host,
        "math",
        "modf",
        &[GuestStorage::Float64, GuestStorage::Pointer],
        Some(GuestStorage::Float64),
        Rc::new(move |ctx, _, args| {
            let value = real(args, 0)?;
            let whole = value.trunc();
            ctx.memory().write_f64(required_pointer(args, 1)?, whole)?;
            let fraction = if value.is_nan() {
                f64::NAN
            } else if value.is_finite() {
                value - whole
            } else {
                0.0
            };
            let result = if fraction == 0.0 && (value < 0.0 || (value == 0.0 && value.is_sign_negative())) {
                -0.0
            } else {
                fraction
            };
            Ok(GuestCallResult::Value(GuestCallValue::Float64(result)))
        }),
    )?;
    for (name, single) in [("_dclass", false), ("_fdclass", true), ("_dsign", false)] {
        service(
            host,
            "math",
            name,
            &[if single {
                GuestStorage::Float32
            } else {
                GuestStorage::Float64
            }],
            Some(GuestStorage::Int16),
            Rc::new(move |_, _, args| {
                let value = real(args, 0)?;
                let code = if name == "_dsign" {
                    if value < 0.0 || (value == 0.0 && value.is_sign_negative()) {
                        0x8000
                    } else {
                        0
                    }
                } else if value.is_nan() {
                    2
                } else if !value.is_finite() {
                    1
                } else if value == 0.0 {
                    0
                } else if value.abs() < if single { 2f64.powi(-126) } else { 2f64.powi(-1022) } {
                    -2
                } else {
                    -1
                };
                Ok(i32_value(if code == 0x8000 { -32768 } else { code }))
            }),
        )?;
    }
    Ok(())
}

/// Longest `parseFloat` numeric prefix of `text`.
fn parse_float_prefix(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() && matches!(bytes[at], b'\t' | b'\n' | 0x0b | 0x0c | b'\r' | b' ') {
        at += 1;
    }
    let rest = &text[at..];
    let signed = rest.strip_prefix('+').or_else(|| rest.strip_prefix('-'));
    let body = signed.unwrap_or(rest);
    if body.starts_with("Infinity") {
        return Some(if rest.starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        });
    }
    let chars: Vec<char> = body.chars().collect();
    let mut index = 0;
    let mut digits = 0;
    while index < chars.len() && chars[index].is_ascii_digit() {
        index += 1;
        digits += 1;
    }
    if index < chars.len() && chars[index] == '.' {
        index += 1;
        while index < chars.len() && chars[index].is_ascii_digit() {
            index += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    let mut end = index;
    if index < chars.len() && matches!(chars[index], 'e' | 'E') {
        let mut probe = index + 1;
        if probe < chars.len() && matches!(chars[probe], '+' | '-') {
            probe += 1;
        }
        let exponent_start = probe;
        while probe < chars.len() && chars[probe].is_ascii_digit() {
            probe += 1;
        }
        if probe > exponent_start {
            end = probe;
        }
    }
    let literal: String = chars[..end].iter().collect();
    let sign = if rest.starts_with('-') { "-" } else { "" };
    format!("{sign}{literal}").parse::<f64>().ok()
}

fn atoi_value(text: &str) -> (u128, bool) {
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() && matches!(bytes[at], b'\t' | b'\n' | 0x0b | 0x0c | b'\r' | b' ') {
        at += 1;
    }
    let negative = bytes.get(at) == Some(&b'-');
    if matches!(bytes.get(at), Some(b'+') | Some(b'-')) {
        at += 1;
    }
    let mut magnitude: u128 = 0;
    while at < bytes.len() && bytes[at].is_ascii_digit() {
        magnitude = magnitude.wrapping_mul(10).wrapping_add((bytes[at] - b'0') as u128);
        at += 1;
    }
    (magnitude, negative)
}

fn install_conversions(host: &mut WindowsServiceRegistrar<'_>) -> Result<(), GuestError> {
    let errno = host.memory.allocate(&crate::core::contracts::GuestAllocationOptions {
        byte_length: 4,
        alignment: 4,
        permissions: crate::core::contracts::GuestPermissions::ReadWrite,
        label: "CRT errno".to_string(),
    })?;
    service(
        host,
        "runtime",
        "_errno",
        &[],
        Some(GuestStorage::Pointer),
        Rc::new(move |_, _, _| Ok(ptr_value(Some(errno)))),
    )?;
    crate::runtime::common::format::services::install_windows_format(host, errno)?;
    service(
        host,
        "convert",
        "strtoul",
        &[GuestStorage::Pointer, GuestStorage::Pointer, GuestStorage::Int32],
        Some(GuestStorage::Uint32),
        Rc::new(move |ctx, _, args| {
            let memory = ctx.memory();
            let source = required_pointer(args, 0)?;
            let end_pointer = pointer(args, 1)?;
            let text = read_string(memory, source, false)?;
            let chars: Vec<char> = text.chars().collect();
            let mut base = integer(args, 2)?;
            let mut index = 0;
            let mut negative = false;
            let finish =
                |memory: &mut SparseGuestMemory, value: u32, end: usize| -> Result<GuestCallResult, GuestError> {
                    if let Some(end_pointer) = end_pointer {
                        memory.write_pointer(end_pointer, Some(memory.offset(source, end as i64)?))?;
                    }
                    Ok(u32_value(value))
                };
            if base != 0 && !(2..=36).contains(&base) {
                memory.write_i32(errno, 22)?;
                return finish(memory, 0, 0);
            }
            while index < chars.len() && " \t\n\r\x0c\x0b".contains(chars[index]) {
                index += 1;
            }
            if chars.get(index) == Some(&'-') || chars.get(index) == Some(&'+') {
                negative = chars[index] == '-';
                index += 1;
            }
            let digit = |chars: &[char], at: usize| -> i128 {
                match chars.get(at) {
                    Some(c) if c.is_ascii_digit() => (*c as u8 - b'0') as i128,
                    Some(c) if c.is_ascii_uppercase() => (*c as u8 - b'A') as i128 + 10,
                    Some(c) if c.is_ascii_lowercase() => (*c as u8 - b'a') as i128 + 10,
                    _ => 99,
                }
            };
            if (base == 0 || base == 16)
                && chars.get(index) == Some(&'0')
                && matches!(chars.get(index + 1), Some('x') | Some('X'))
                && digit(&chars, index + 2) < 16
            {
                base = 16;
                index += 2;
            }
            if base == 0 {
                base = if chars.get(index) == Some(&'0') { 8 } else { 10 };
            }
            let first = index;
            let mut value: u64 = 0;
            let mut overflowed = false;
            while digit(&chars, index) < base {
                value = value
                    .saturating_mul(base as u64)
                    .saturating_add(digit(&chars, index) as u64);
                if value > 0xffff_ffff {
                    overflowed = true;
                    value = 0xffff_ffff;
                }
                index += 1;
            }
            if index == first {
                return finish(memory, 0, 0);
            }
            if overflowed {
                memory.write_i32(errno, 34)?;
                return finish(memory, 0xffff_ffff, index);
            }
            let result = if negative {
                0u32.wrapping_sub(value as u32)
            } else {
                value as u32
            };
            finish(memory, result, index)
        }),
    )?;
    service(
        host,
        "convert",
        "atoi",
        &[GuestStorage::Pointer],
        Some(GuestStorage::Int32),
        Rc::new(move |ctx, _, args| {
            let text = read_string(ctx.memory(), required_pointer(args, 0)?, false)?;
            let (magnitude, negative) = atoi_value(&text);
            let low = magnitude as u32;
            Ok(i32_value(if negative {
                0u32.wrapping_sub(low) as i32
            } else {
                low as i32
            }))
        }),
    )?;
    service(
        host,
        "convert",
        "atoll",
        &[GuestStorage::Pointer],
        Some(GuestStorage::Int64),
        Rc::new(move |ctx, _, args| {
            let text = read_string(ctx.memory(), required_pointer(args, 0)?, false)?;
            let (magnitude, negative) = atoi_value(&text);
            let low = magnitude as u64;
            Ok(GuestCallResult::Value(GuestCallValue::Int64(if negative {
                0u64.wrapping_sub(low) as i64
            } else {
                low as i64
            })))
        }),
    )?;
    service(
        host,
        "convert",
        "atof",
        &[GuestStorage::Pointer],
        Some(GuestStorage::Float64),
        Rc::new(move |ctx, _, args| {
            let text = read_string(ctx.memory(), required_pointer(args, 0)?, false)?;
            let value = parse_float_prefix(&text).unwrap_or(0.0);
            Ok(GuestCallResult::Value(GuestCallValue::Float64(if value.is_nan() {
                0.0
            } else {
                value
            })))
        }),
    )?;
    {
        let shared = Rc::clone(&host.shared);
        service(
            host,
            "time",
            "_time64",
            &[GuestStorage::Pointer],
            Some(GuestStorage::Int64),
            Rc::new(move |ctx, _, args| {
                let value = now_millis(&shared) / 1000;
                if let Some(out) = pointer(args, 0)? {
                    ctx.memory().write_i64(out, value)?;
                }
                Ok(GuestCallResult::Value(GuestCallValue::Int64(value)))
            }),
        )?;
    }
    service(
        host,
        "time",
        "_localtime64",
        &[GuestStorage::Pointer],
        Some(GuestStorage::Pointer),
        Rc::new(move |ctx, _, args| {
            use crate::runtime::common::memory::read_unsigned;
            let memory = ctx.memory();
            let value = read_unsigned(memory, required_pointer(args, 0)?, 8)? as i64;
            if !(0..=32_535_215_999).contains(&value) {
                return Ok(ptr_value(None));
            }
            let slot = memory.offset(required_pointer(args, 0)?, 8)?;
            let (offset, dst) = local_offset_at(value);
            let fields = tm_fields(value, offset, dst);
            for (index, field) in fields.iter().enumerate() {
                memory.write_i32(memory.offset(slot, index as i64 * 4)?, *field)?;
            }
            Ok(ptr_value(Some(slot)))
        }),
    )?;
    {
        let width = host.pointer_bytes;
        let empty = host.memory.allocate(&crate::core::contracts::GuestAllocationOptions {
            byte_length: 1,
            alignment: 1,
            permissions: crate::core::contracts::GuestPermissions::ReadWrite,
            label: "CRT locale string".to_string(),
        })?;
        let decimal = host.memory.allocate(&crate::core::contracts::GuestAllocationOptions {
            byte_length: 2,
            alignment: 1,
            permissions: crate::core::contracts::GuestPermissions::ReadWrite,
            label: "CRT locale string".to_string(),
        })?;
        host.memory.write(decimal, b".\0")?;
        let locale = host.memory.allocate(&crate::core::contracts::GuestAllocationOptions {
            byte_length: width * 10 + 16,
            alignment: 16,
            permissions: crate::core::contracts::GuestPermissions::ReadWrite,
            label: "CRT locale block".to_string(),
        })?;
        for index in 0..10 {
            host.memory.write_pointer(
                host.memory.offset(locale, (index * width) as i64)?,
                Some(if index == 0 { decimal } else { empty }),
            )?;
        }
        host.memory
            .write(host.memory.offset(locale, (width * 10) as i64)?, &[127u8; 14])?;
        service(
            host,
            "locale",
            "localeconv",
            &[],
            Some(GuestStorage::Pointer),
            Rc::new(move |_, _, _| Ok(ptr_value(Some(locale)))),
        )?;
    }
    Ok(())
}
