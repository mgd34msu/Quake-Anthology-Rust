//! MSVC locale services: facets, lockits, and locale info.
//!
//! Donor: `src/guest/runtime/windows/msvc/locale.ts` (MSVC x64
//! xlocale/xfacet layout).

use std::rc::Rc;

use crate::core::contracts::{
    GuestAddress, GuestAllocationOptions, GuestCallResult, GuestCallValue, GuestPermissions,
    GuestStorage,
};
use crate::core::memory::SparseGuestMemory;
use crate::error::GuestError;
use crate::runtime::common::memory::{integer, pointer, read_string, required_pointer, string_bytes};
use crate::runtime::windows::contracts::{
    invoke_nested, unsupported_windows, WindowsContext, WindowsServiceRegistrar,
};

const LIBRARY: &str = "msvcp140.dll";

/// Constructed MSVC locale with its shared facet state.
#[derive(Debug, Clone)]
pub struct MsvcLocale {
    /// Global locale object.
    pub global: GuestAddress,
    facet_vtable: GuestAddress,
    locks: GuestAddress,
    context: WindowsContext,
    teb: GuestAddress,
    thread_id: u32,
}

impl MsvcLocale {
    /// Allocate tracked memory; fails when the heap is exhausted.
    pub fn allocate(&self, memory: &mut SparseGuestMemory, size: usize) -> Result<GuestAddress, GuestError> {
        self.context.allocate(memory, self.teb, size, 0)?.ok_or_else(|| {
            GuestError::invalid("MSVC allocation failed")
        })
    }

    /// Retain a facet.
    pub fn incref(&self, memory: &mut SparseGuestMemory, facet: GuestAddress) -> Result<(), GuestError> {
        let slot = memory.offset(facet, 8)?;
        let count = memory.read_u32(slot)?;
        memory.write_u32(slot, count + 1)?;
        Ok(())
    }

    /// Release a facet; returns it when the last reference drops.
    pub fn decref(
        &self,
        memory: &mut SparseGuestMemory,
        facet: GuestAddress,
    ) -> Result<Option<GuestAddress>, GuestError> {
        let slot = memory.offset(facet, 8)?;
        let refs = memory.read_u32(slot)?;
        if refs == 0 {
            return Err(GuestError::callback("MSVC facet reference underflow"));
        }
        memory.write_u32(slot, refs - 1)?;
        Ok(if refs == 1 { Some(facet) } else { None })
    }

    /// Allocate a locale handle retaining the global locale.
    pub fn create(&self, memory: &mut SparseGuestMemory) -> Result<GuestAddress, GuestError> {
        let result = self.allocate(memory, 8)?;
        self.incref(memory, self.global)?;
        memory.write_pointer(result, Some(self.global))?;
        Ok(result)
    }

    /// Release a locale handle.
    pub fn destroy(
        &self,
        memory: &mut SparseGuestMemory,
        locale: Option<GuestAddress>,
    ) -> Result<(), GuestError> {
        let Some(locale) = locale else {
            return Ok(());
        };
        let facet = memory
            .read_pointer(locale)?
            .ok_or_else(|| GuestError::callback("Missing locale implementation"))?;
        // The process C locale has the same reference protocol as guest-created facets.
        if facet.offset != self.global.offset {
            return Err(unsupported_windows(
                LIBRARY,
                "locale destructor",
                "non-C locale implementation destructor is not implemented",
            ));
        }
        if self.decref(memory, facet)?.is_some() {
            return Err(GuestError::callback(
                "MSVC process locale lost its owner references",
            ));
        }
        if !self.context.free(memory, self.teb, Some(locale), 0)? {
            return Err(GuestError::callback("Invalid MSVC locale allocation"));
        }
        Ok(())
    }

    /// Enter a locale lock.
    pub fn lock(&self, memory: &mut SparseGuestMemory, object: GuestAddress, kind: i64) -> Result<(), GuestError> {
        memory.write_i32(object, kind as i32)?;
        if kind < 0 || kind >= 8 {
            // _Lockit deliberately ignores out-of-range categories.
            return Ok(());
        }
        let slot = memory.offset(self.locks, kind * 8)?;
        let owner = memory.read_u32(slot)?;
        let depth_slot = memory.offset(slot, 4)?;
        if owner != 0 && owner != self.thread_id {
            return Err(GuestError::callback("MSVC lock contention requires thread scheduling"));
        }
        memory.write_u32(slot, self.thread_id)?;
        let depth = memory.read_u32(depth_slot)?;
        memory.write_u32(depth_slot, depth + 1)?;
        Ok(())
    }

    /// Leave a locale lock.
    pub fn unlock(&self, memory: &mut SparseGuestMemory, object: GuestAddress) -> Result<(), GuestError> {
        let kind = memory.read_i32(object)?;
        if kind < 0 || kind >= 8 {
            return Ok(());
        }
        let slot = memory.offset(self.locks, i64::from(kind) * 8)?;
        let depth_slot = memory.offset(slot, 4)?;
        let depth = memory.read_u32(depth_slot)?;
        if memory.read_u32(slot)? != self.thread_id || depth == 0 {
            return Err(GuestError::callback("Unowned MSVC lock release"));
        }
        memory.write_u32(depth_slot, depth - 1)?;
        if depth == 1 {
            memory.write_u32(slot, 0)?;
        }
        Ok(())
    }
}

fn direct(
    memory: &mut SparseGuestMemory,
    byte_length: usize,
    label: &str,
) -> Result<GuestAddress, GuestError> {
    memory.allocate(&GuestAllocationOptions {
        byte_length,
        alignment: 16,
        permissions: GuestPermissions::ReadWrite,
        label: label.to_string(),
    })
}

/// Register the MSVC locale service set; returns the constructed locale.
pub fn install_msvc_locale(host: &mut WindowsServiceRegistrar<'_>) -> Result<MsvcLocale, GuestError> {
    let width = host.pointer_bytes;
    let context = host.context.clone();
    let shared = Rc::clone(&host.shared);
    let locks = direct(host.memory, 64, "MSVC recursive library locks")?;
    // Addresses are final up front; guest-memory filling below runs before
    // any registered closure can execute.
    let facet_vtable = direct(host.memory, 24, "MSVC facet vftable")?;
    let global = direct(host.memory, 56, "MSVC C locale implementation")?;
    let locale = MsvcLocale {
        global,
        facet_vtable,
        locks,
        context: context.clone(),
        teb: host.teb,
        thread_id: host.thread_id,
    };
    {
        let locale = locale.clone();
        host.service(LIBRARY, "?_Incref@facet@locale@std@@UEAAXXZ", &[GuestStorage::Pointer], None, Rc::new(
            move |ctx, _, args| {
                locale.incref(ctx.memory(), required_pointer(args, 0)?)?;
                Ok(GuestCallResult::Void)
            },
        ))?;
    }
    {
        let locale = locale.clone();
        host.service(LIBRARY, "?_Decref@facet@locale@std@@UEAAPEAV_Facet_base@3@XZ", &[GuestStorage::Pointer], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let value = locale.decref(ctx.memory(), required_pointer(args, 0)?)?;
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(value)))
            },
        ))?;
    }
    {
        let locale = locale.clone();
        host.service(LIBRARY, "??1facet@locale@std@@MEAA@XZ", &[GuestStorage::Pointer], None, Rc::new(
            move |ctx, _, args| {
                ctx.memory().write_pointer(required_pointer(args, 0)?, Some(locale.facet_vtable))?;
                Ok(GuestCallResult::Void)
            },
        ))?;
    }
    {
        let locale = locale.clone();
        let runtime = context.clone();
        let teb = host.teb;
        host.service("msvcp140.dll", "runtime:facet-delete", &[GuestStorage::Pointer, GuestStorage::Uint32], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let facet = required_pointer(args, 0)?;
                if facet.offset == locale.global.offset {
                    return Err(unsupported_windows(
                        LIBRARY,
                        "locale deletion",
                        "the process classic/global locale still owns references",
                    ));
                }
                let memory = ctx.memory();
                memory.write_pointer(facet, Some(locale.facet_vtable))?;
                if integer(args, 1)? & 1 != 0 && !runtime.free(memory, teb, Some(facet), 0)? {
                    return Err(GuestError::callback("Invalid MSVC facet allocation"));
                }
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(facet))))
            },
        ))?;
    }
    for (index, name) in [
        "runtime:facet-delete",
        "?_Incref@facet@locale@std@@UEAAXXZ",
        "?_Decref@facet@locale@std@@UEAAPEAV_Facet_base@3@XZ",
    ]
    .iter()
    .enumerate()
    {
        let address = context
            .resolve_address(LIBRARY, name)
            .ok_or_else(|| GuestError::callback("Missing MSVC facet method"))?;
        host.memory.write_pointer(host.memory.offset(facet_vtable, (index * 8) as i64)?, Some(address))?;
    }
    host.memory.protect(facet_vtable, 24, GuestPermissions::Read)?;
    // Classic() and the process global locale.
    host.memory.write_pointer(global, Some(facet_vtable))?;
    host.memory.write_u32(host.memory.offset(global, 8)?, 2)?;
    host.memory.write_u32(host.memory.offset(global, 32)?, 63)?;
    let name = direct(host.memory, 2, "MSVC locale name")?;
    host.memory.write(name, &string_bytes("C", false))?;
    host.memory.write_pointer(host.memory.offset(global, 40)?, Some(name))?;
    {
        let locale = locale.clone();
        host.service(LIBRARY, "??0facet@locale@std@@IEAA@_K@Z", &[GuestStorage::Pointer, GuestStorage::Uint64], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let facet = required_pointer(args, 0)?;
                let memory = ctx.memory();
                memory.write_pointer(facet, Some(locale.facet_vtable))?;
                memory.write_u32(memory.offset(facet, 8)?, integer(args, 1)? as u32)?;
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(facet))))
            },
        ))?;
    }
    {
        let locale = locale.clone();
        host.service(LIBRARY, "?_Init@locale@std@@CAPEAV_Locimp@12@_N@Z", &[GuestStorage::Uint32], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                if integer(args, 0)? & 255 != 0 {
                    locale.incref(ctx.memory(), locale.global)?;
                }
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(locale.global))))
            },
        ))?;
    }
    {
        let locale = locale.clone();
        host.service(LIBRARY, "?_Getgloballocale@locale@std@@CAPEAV_Locimp@12@XZ", &[], Some(GuestStorage::Pointer), Rc::new(
            move |_, _, _| Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(locale.global)))),
        ))?;
    }
    {
        let locale = locale.clone();
        host.service(LIBRARY, "??0_Lockit@std@@QEAA@H@Z", &[GuestStorage::Pointer, GuestStorage::Int32], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let object = required_pointer(args, 0)?;
                locale.lock(ctx.memory(), object, integer(args, 1)? as i64)?;
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(object))))
            },
        ))?;
    }
    {
        let locale = locale.clone();
        host.service(LIBRARY, "??1_Lockit@std@@QEAA@XZ", &[GuestStorage::Pointer], None, Rc::new(
            move |ctx, _, args| {
                locale.unlock(ctx.memory(), required_pointer(args, 0)?)?;
                Ok(GuestCallResult::Void)
            },
        ))?;
    }
    {
        let locale = locale.clone();
        host.service(LIBRARY, "??0_Locinfo@std@@QEAA@PEBD@Z", &[GuestStorage::Pointer, GuestStorage::Pointer], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let memory = ctx.memory();
                let object = required_pointer(args, 0)?;
                let supplied = pointer(args, 1)?;
                let requested = match supplied {
                    None => None,
                    Some(supplied) => Some(read_string(memory, supplied, false)?),
                };
                if requested.as_deref() != Some("C") && requested.as_deref() != Some("") {
                    return Err(unsupported_windows(
                        LIBRARY,
                        "_Locinfo",
                        format!(
                            "locale {} is not implemented",
                            requested.as_deref().unwrap_or("null")
                        ),
                    ));
                }
                memory.write(object, &[0u8; 104])?;
                locale.lock(memory, object, 0)?;
                for (offset, wide) in [(72, true), (88, false)] {
                    let bytes = string_bytes("C", wide);
                    let address = locale.allocate(memory, bytes.len())?;
                    memory.write(address, &bytes)?;
                    memory.write_pointer(memory.offset(object, offset)?, Some(address))?;
                }
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(object))))
            },
        ))?;
    }
    {
        let locale = locale.clone();
        host.service(LIBRARY, "??1_Locinfo@std@@QEAA@XZ", &[GuestStorage::Pointer], None, Rc::new(
            move |ctx, _, args| {
                let memory = ctx.memory();
                let object = required_pointer(args, 0)?;
                let mut offset = 8;
                while offset <= 88 {
                    let slot = memory.offset(object, offset)?;
                    let allocation = memory.read_pointer(slot)?;
                    if !locale.context.free(memory, locale.teb, allocation, 0)? {
                        return Err(GuestError::callback("Invalid _Locinfo yarn allocation"));
                    }
                    memory.write_pointer(slot, None)?;
                    offset += 16;
                }
                locale.unlock(memory, object)?;
                Ok(GuestCallResult::Void)
            },
        ))?;
    }
    for word in ["true", "false"] {
        let bytes = string_bytes(word, false);
        let address = direct(host.memory, bytes.len(), &format!("MSVC {word} name"))?;
        host.memory.write(address, &bytes)?;
        host.service(
            LIBRARY,
            &format!("?_Get{word}@_Locinfo@std@@QEBAPEBDXZ"),
            &[GuestStorage::Pointer],
            Some(GuestStorage::Pointer),
            Rc::new(move |_, _, _| {
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(address))))
            }),
        )?;
    }
    {
        let shared = Rc::clone(&shared);
        let runtime = context.clone();
        host.service(LIBRARY, "?_Getlconv@_Locinfo@std@@QEBAPEBUlconv@@XZ", &[GuestStorage::Pointer], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, context, _| {
                let target = runtime.resolve_address("ucrtbase.dll", "localeconv").ok_or_else(|| {
                    GuestError::callback("CRT localeconv missing")
                })?;
                invoke_nested(ctx, &shared, width, context, target, &[], Some(GuestStorage::Pointer), vec![], false)
            },
        ))?;
    }
    host.service(LIBRARY, "?_Getcvt@_Locinfo@std@@QEBA?AU_Cvtvec@@XZ", &[GuestStorage::Pointer, GuestStorage::Pointer], Some(GuestStorage::Pointer), Rc::new(
        move |ctx, _, args| {
            // _Cvtvec is returned through the member-function hidden result pointer in RDX.
            let memory = ctx.memory();
            let result = required_pointer(args, 1)?;
            memory.write(result, &[0u8; 44])?;
            memory.write_u32(memory.offset(result, 4)?, 1)?;
            memory.write_u32(memory.offset(result, 8)?, 1)?;
            Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(result))))
        },
    ))?;
    Ok(locale)
}
