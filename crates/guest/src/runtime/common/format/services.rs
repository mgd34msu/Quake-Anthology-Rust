//! `printf`/`scanf` service installers shared by both runtimes.
//!
//! Donor: `src/guest/runtime/common/format/services.ts`.

use std::rc::Rc;

use crate::core::contracts::{GuestAddress, GuestCallResult, GuestCallValue, GuestStorage};
use crate::error::GuestError;
use crate::floating_point::binary::rounding;
use crate::runtime::common::format::arguments::FormatDialect;
use crate::runtime::common::format::float::FormatRounding;
use crate::runtime::common::format::scan::scan_windows_buffer;
use crate::runtime::common::format::{format_guest_buffer, FormatTermination, GuestFormatRequest};
use crate::runtime::common::memory::{integer, pointer, required_pointer};

/// Install the UCRT `__stdio_common_*` entry points.
pub fn install_windows_format(
    host: &mut crate::runtime::windows::contracts::WindowsServiceRegistrar<'_>,
    errno: GuestAddress,
) -> Result<(), GuestError> {
    let pointer_storage = host.pointer_storage();
    for library in [
        "api-ms-win-crt-stdio-l1-1-0.dll",
        "ucrtbase.dll",
        "msvcrt.dll",
    ] {
        let library_owned = library.to_string();
        host.service(
            library,
            "__stdio_common_vsscanf",
            &[
                GuestStorage::Uint64,
                GuestStorage::Pointer,
                pointer_storage,
                GuestStorage::Pointer,
                GuestStorage::Pointer,
                GuestStorage::Pointer,
            ],
            Some(GuestStorage::Int32),
            Rc::new(move |ctx, context, args| {
                let _ = context;
                if pointer(args, 4)? != None || (integer(args, 0)? & !2) != 0 {
                    return Err(crate::runtime::windows::contracts::unsupported_windows(
                        &library_owned,
                        "__stdio_common_vsscanf",
                        "scanf locale or options are not implemented",
                    ));
                }
                let memory = ctx.memory();
                let capacity = integer(args, 2)?;
                let input = required_pointer(args, 1)?;
                let format = required_pointer(args, 3)?;
                let arguments = pointer(args, 5)?;
                match scan_windows_buffer(memory, input, capacity, format, arguments) {
                    Ok(assigned) => Ok(GuestCallResult::Value(GuestCallValue::Int32(assigned))),
                    Err(error) => Err(crate::runtime::windows::contracts::unsupported_windows(
                        &library_owned,
                        "__stdio_common_vsscanf",
                        error.to_string(),
                    )),
                }
            }),
        )?;
    }
    for library in [
        "api-ms-win-crt-stdio-l1-1-0.dll",
        "ucrtbase.dll",
        "msvcrt.dll",
    ] {
        let library_owned = library.to_string();
        host.service(
            library,
            "__stdio_common_vsprintf",
            &[
                GuestStorage::Uint64,
                GuestStorage::Pointer,
                pointer_storage,
                GuestStorage::Pointer,
                GuestStorage::Pointer,
                GuestStorage::Pointer,
            ],
            Some(GuestStorage::Int32),
            Rc::new(move |ctx, context, args| {
                let _ = context;
                let options = integer(args, 0)?;
                if pointer(args, 4)? != None {
                    return Err(crate::runtime::windows::contracts::unsupported_windows(
                        &library_owned,
                        "__stdio_common_vsprintf",
                        "explicit printf locale is not the installed C locale",
                    ));
                }
                if (options & !0x3f) != 0 || (options & 8) != 0 {
                    return Err(crate::runtime::windows::contracts::unsupported_windows(
                        &library_owned,
                        "__stdio_common_vsprintf",
                        "legacy MSVCRT compatibility formatting is not implemented",
                    ));
                }
                let capacity = integer(args, 2)?;
                if capacity < 0 || capacity > u64::MAX as i128 {
                    return Err(GuestError::invalid(
                        "Guest printf buffer count is not size_t",
                    ));
                }
                let rounding = if (options & 32) != 0 {
                    let mxcsr = ctx.cpu_state().simd.mxcsr;
                    FormatRounding::Ieee(rounding(mxcsr >> 13))
                } else {
                    FormatRounding::LegacyNearest
                };
                let buffer = pointer(args, 1)?;
                let format = pointer(args, 3)?;
                let arguments = pointer(args, 5)?;
                let mut request = GuestFormatRequest {
                    memory: ctx.memory(),
                    dialect: FormatDialect::Windows,
                    format,
                    arguments,
                    buffer,
                    capacity: capacity as u64,
                    termination: if (options & 1) != 0 {
                        FormatTermination::UcrtLegacy
                    } else if (options & 2) != 0 {
                        FormatTermination::C99
                    } else {
                        FormatTermination::Ucrt
                    },
                    continue_count: (options & 2) != 0,
                    rounding,
                    exponent_digits: if (options & 16) != 0 { 3 } else { 2 },
                    fortify: false,
                };
                let result = format_guest_buffer(&mut request)?;
                if let Some(errno_value) = result.errno {
                    request.memory.write_i32(errno, errno_value)?;
                }
                Ok(GuestCallResult::Value(GuestCallValue::Int32(result.result)))
            }),
        )?;
    }
    Ok(())
}

/// Install glibc `vsnprintf` and `__vsnprintf_chk`.
pub fn install_system_v_format(
    host: &mut crate::runtime::system_v::contracts::SystemVServiceRegistrar<'_>,
) -> Result<(), GuestError> {
    let pointer_storage = host.pointer_storage();
    let base = if host.pointer_bytes == 4 {
        "GLIBC_2.0"
    } else {
        "GLIBC_2.2.5"
    };
    for checked in [false, true] {
        let name = if checked { "__vsnprintf_chk" } else { "vsnprintf" };
        let versions: [Option<&str>; 2] =
            [Some(if checked { "GLIBC_2.3.4" } else { base }), None];
        let parameters = if checked {
            vec![
                GuestStorage::Pointer,
                pointer_storage,
                GuestStorage::Int32,
                pointer_storage,
                GuestStorage::Pointer,
                GuestStorage::Pointer,
            ]
        } else {
            vec![
                GuestStorage::Pointer,
                pointer_storage,
                GuestStorage::Pointer,
                GuestStorage::Pointer,
            ]
        };
        let errno_address = host.errno_address;
        host.service(
            "libc.so.6",
            name,
            &versions,
            &parameters,
            Some(GuestStorage::Int32),
            Rc::new(move |ctx, _, args| {
                let capacity = integer(args, 1)?;
                if capacity < 0 || capacity > u64::MAX as i128 {
                    return Err(GuestError::invalid(
                        "Guest printf buffer count is not size_t",
                    ));
                }
                if checked && integer(args, 3)? < capacity {
                    return Err(GuestError::invalid(
                        "glibc __vsnprintf_chk destination size is smaller than maxlen",
                    ));
                }
                // glibc clears the first byte before processing, including
                // empty and overlapping formats.
                if capacity > 0 {
                    let memory = ctx.memory();
                    let buffer = required_pointer(args, 0)?;
                    memory.write_u8(buffer, 0)?;
                }
                let rounding = {
                    let control = ctx.cpu_state().x87.control_word;
                    FormatRounding::Ieee(rounding(u32::from(control) >> 10))
                };
                let buffer = pointer(args, 0)?;
                let format = pointer(args, if checked { 4 } else { 2 })?;
                let arguments = pointer(args, if checked { 5 } else { 3 })?;
                let fortify = checked && integer(args, 2)? > 0;
                let mut request = GuestFormatRequest {
                    memory: ctx.memory(),
                    dialect: FormatDialect::SystemV,
                    format,
                    arguments,
                    buffer,
                    capacity: capacity as u64,
                    termination: FormatTermination::C99,
                    continue_count: false,
                    rounding,
                    exponent_digits: 2,
                    fortify,
                };
                let result = format_guest_buffer(&mut request)?;
                if let Some(errno_value) = result.errno {
                    request.memory.write_i32(errno_address, errno_value)?;
                }
                Ok(GuestCallResult::Value(GuestCallValue::Int32(result.result)))
            }),
        )?;
    }
    Ok(())
}
