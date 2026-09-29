//! System V C++ runtime: guards, `new`/`delete`, exceptions, destructors.
//!
//! Donor: `src/guest/runtime/system-v/cxx.ts`.

use std::rc::Rc;

use crate::core::contracts::{GuestAddress, GuestCallContext, GuestCallResult, GuestCallValue, GuestStorage};
use crate::error::GuestError;
use crate::runtime::common::memory::{count, pointer, required_pointer};
use crate::runtime::system_v::contracts::{
    invoke_nested, unsupported_system_v, SharedSystemV, SystemVDestructor, SystemVServiceRegistrar,
};
use crate::runtime::system_v::libc::{system_v_allocate, system_v_free};

/// Register the C++ service set.
pub fn install_cxx(host: &mut SystemVServiceRegistrar<'_>) -> Result<(), GuestError> {
    let pointer_bytes = host.pointer_bytes;
    let pointer_storage = host.pointer_storage();
    let cxx = "libstdc++.so.6";
    let cxa: [Option<&str>; 2] = [Some("CXXABI_1.3"), None];
    let old: [Option<&str>; 2] = [Some("GLIBCXX_3.4"), None];
    let libc_version = if pointer_bytes == 4 {
        "GLIBC_2.1.3"
    } else {
        "GLIBC_2.2.5"
    };
    let shared = Rc::clone(&host.shared);

    {
        let shared = Rc::clone(&shared);
        host.service(
            "libc.so.6",
            "__cxa_atexit",
            &[Some(libc_version), None],
            &[GuestStorage::Pointer, GuestStorage::Pointer, GuestStorage::Pointer],
            Some(GuestStorage::Int32),
            Rc::new(move |ctx, _, args| {
                let target = required_pointer(args, 0)?;
                ctx.memory()
                    .check(target, 1, crate::core::contracts::GuestAccess::Execute)?;
                shared.borrow_mut().destructors.push(SystemVDestructor {
                    target,
                    argument: pointer(args, 1)?,
                    dso: pointer(args, 2)?,
                    called: false,
                });
                Ok(GuestCallResult::Value(GuestCallValue::Int32(0)))
            }),
        )?;
    }
    {
        let shared = Rc::clone(&shared);
        host.service(
            "libc.so.6",
            "__cxa_finalize",
            &[Some(libc_version), None],
            &[GuestStorage::Pointer],
            None,
            Rc::new(move |ctx, context, args| {
                finalize_destructors(ctx, &shared, pointer_bytes, context, pointer(args, 0)?)?;
                Ok(GuestCallResult::Void)
            }),
        )?;
    }
    host.service(
        cxx,
        "__cxa_guard_acquire",
        &cxa,
        &[GuestStorage::Pointer],
        Some(GuestStorage::Int32),
        Rc::new(move |ctx, context, args| {
            let _ = context;
            let memory = ctx.memory();
            let guard = required_pointer(args, 0)?;
            if memory.read_u8(guard)? != 0 {
                return Ok(GuestCallResult::Value(GuestCallValue::Int32(0)));
            }
            if memory.read_u8(memory.offset(guard, 1)?)? != 0 {
                return Err(unsupported_system_v(
                    cxx,
                    "__cxa_guard_acquire",
                    Some("CXXABI_1.3"),
                    "recursive local static initialization",
                ));
            }
            memory.write_u8(memory.offset(guard, 1)?, 1)?;
            Ok(GuestCallResult::Value(GuestCallValue::Int32(1)))
        }),
    )?;
    host.service(
        cxx,
        "__cxa_guard_release",
        &cxa,
        &[GuestStorage::Pointer],
        None,
        Rc::new(move |ctx, _, args| {
            let memory = ctx.memory();
            let guard = required_pointer(args, 0)?;
            memory.write_u8(guard, 1)?;
            memory.write_u8(memory.offset(guard, 1)?, 0)?;
            Ok(GuestCallResult::Void)
        }),
    )?;
    host.service(
        cxx,
        "__cxa_guard_abort",
        &cxa,
        &[GuestStorage::Pointer],
        None,
        Rc::new(move |ctx, _, args| {
            let memory = ctx.memory();
            memory.write_u8(memory.offset(required_pointer(args, 0)?, 1)?, 0)?;
            Ok(GuestCallResult::Void)
        }),
    )?;
    for name in [
        if pointer_bytes == 4 { "_Znwj" } else { "_Znwm" },
        if pointer_bytes == 4 { "_Znaj" } else { "_Znam" },
    ] {
        let shared = Rc::clone(&shared);
        host.service(
            cxx,
            name,
            &old,
            &[pointer_storage],
            Some(GuestStorage::Pointer),
            Rc::new(move |ctx, _, args| {
                let address = system_v_allocate(&shared, ctx.memory(), count(args, 0)?)?;
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(address))))
            }),
        )?;
    }
    for name in ["_ZdlPv", "_ZdaPv"] {
        let shared = Rc::clone(&shared);
        host.service(
            cxx,
            name,
            &old,
            &[GuestStorage::Pointer],
            None,
            Rc::new(move |ctx, _, args| {
                system_v_free(&shared, ctx.memory(), pointer(args, 0)?)?;
                Ok(GuestCallResult::Void)
            }),
        )?;
    }
    // An exception allocation includes raw Itanium/GNU bookkeeping before the user object.
    let exception_header: i64 = if pointer_bytes == 4 { 96 } else { 128 };
    {
        let shared = Rc::clone(&shared);
        host.service(
            cxx,
            "__cxa_allocate_exception",
            &cxa,
            &[pointer_storage],
            Some(GuestStorage::Pointer),
            Rc::new(move |ctx, _, args| {
                let memory = ctx.memory();
                let allocation = system_v_allocate(&shared, memory, count(args, 0)? + exception_header as usize)?;
                let address = memory.offset(allocation, exception_header)?;
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(address))))
            }),
        )?;
    }
    {
        let shared = Rc::clone(&shared);
        host.service(
            cxx,
            "__cxa_free_exception",
            &cxa,
            &[GuestStorage::Pointer],
            None,
            Rc::new(move |ctx, _, args| {
                let memory = ctx.memory();
                let object = required_pointer(args, 0)?;
                system_v_free(&shared, memory, Some(memory.offset(object, -exception_header)?))?;
                Ok(GuestCallResult::Void)
            }),
        )?;
    }
    Ok(())
}

/// Run registered destructors, newest first, optionally filtered by DSO.
pub fn finalize_destructors(
    ctx: &mut crate::core::callbacks::HostCallContext<'_, '_>,
    shared: &SharedSystemV,
    pointer_bytes: usize,
    context: &GuestCallContext,
    dso: Option<GuestAddress>,
) -> Result<(), GuestError> {
    loop {
        let next = {
            let mut shared = shared.borrow_mut();
            let index = shared.destructors.iter().rposition(|entry| {
                !entry.called && (dso.is_none() || entry.dso.map(|dso_| dso_.offset) == dso.map(|dso_| dso_.offset))
            });
            match index {
                Some(index) => {
                    shared.destructors[index].called = true;
                    let entry = shared.destructors[index];
                    Some((entry.target, entry.argument))
                }
                None => None,
            }
        };
        let Some((target, argument)) = next else {
            return Ok(());
        };
        invoke_nested(
            ctx,
            shared,
            pointer_bytes,
            context,
            target,
            &[GuestStorage::Pointer],
            None,
            vec![GuestCallValue::Pointer(argument)],
        )?;
    }
}
