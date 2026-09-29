//! Itanium/GNU C++ ABI data representation (libstdc++ 4.8).
//!
//! Donor: `src/guest/runtime/system-v/cxx-data.ts`. Helpers take an explicit
//! service context and guest memory so both construction and the runtime
//! `ios_base::Init` path can build RTTI.

use std::cell::RefCell;
use std::collections::HashMap;

use std::rc::Rc;

use crate::core::callbacks::HostCallbackFn;
use crate::core::contracts::{GuestAddress, GuestCallResult, GuestCallValue, GuestStorage};
use crate::core::memory::SparseGuestMemory;
use crate::error::GuestError;
use crate::runtime::common::memory::{string_bytes, write_unsigned};
use crate::runtime::system_v::contracts::SystemVContext;

/// One RTTI base: type address, encoded offset, and flags.
#[derive(Debug, Clone, Copy)]
pub struct CxxBase {
    /// Base type address.
    pub address: GuestAddress,
    /// Encoded base offset.
    pub offset: i64,
    /// Base flags.
    pub flags: u64,
}

/// Constructed C++ ABI data: RTTI types and class vtables.
#[derive(Debug, Default)]
pub struct CxxAbiData {
    /// Typeinfo addresses by mangled name.
    pub types: HashMap<String, GuestAddress>,
    /// Class-info vtable addresses by form.
    pub class_info_vtables: HashMap<String, GuestAddress>,
}

/// Shared handle to [`CxxAbiData`].
pub type SharedAbi = Rc<RefCell<CxxAbiData>>;

/// Guest address plus `offset` bytes.
pub fn abi_slot(
    memory: &SparseGuestMemory,
    address: GuestAddress,
    offset: i64,
) -> Result<GuestAddress, GuestError> {
    memory.offset(address, offset)
}

/// Allocate and write a guest copy of `text`.
pub fn abi_bytes(
    ctx: &SystemVContext,
    memory: &mut SparseGuestMemory,
    text: &str,
) -> Result<GuestAddress, GuestError> {
    let bytes = string_bytes(text, false);
    let address = ctx.allocate(memory, bytes.len())?;
    memory.write(address, &bytes)?;
    Ok(address)
}

/// Register a C++ service; returns its trap address.
pub fn abi_function(
    ctx: &SystemVContext,
    memory: &mut SparseGuestMemory,
    name: &str,
    parameters: &[GuestStorage],
    result: Option<GuestStorage>,
    invoke: HostCallbackFn,
    version: &str,
) -> Result<GuestAddress, GuestError> {
    ctx.service(memory, "libstdc++.so.6", name, &[Some(version), None], parameters, result, invoke)?;
    ctx.resolve_address("libstdc++.so.6", name, Some(version))
        .ok_or_else(|| GuestError::callback(format!("Unbound C++ service {name}")))
}

/// Register an always-unsupported C++ trap; returns its address.
pub fn abi_unsupported(
    ctx: &SystemVContext,
    memory: &mut SparseGuestMemory,
    name: &str,
) -> Result<GuestAddress, GuestError> {
    ctx.unavailable(
        memory,
        "libstdc++.so.6",
        name,
        &[GuestStorage::Pointer],
        None,
        "this C++ virtual operation is not implemented",
    )
}

/// Allocate and register C++ data.
pub fn abi_data(
    ctx: &SystemVContext,
    memory: &mut SparseGuestMemory,
    name: &str,
    size: usize,
    version: &str,
) -> Result<GuestAddress, GuestError> {
    let address = ctx.allocate(memory, size)?;
    ctx.data(memory, "libstdc++.so.6", name, Some(version), address, size)?;
    Ok(address)
}

/// Build the root RTTI graph.
pub fn build_abi(
    ctx: &SystemVContext,
    memory: &mut SparseGuestMemory,
) -> Result<CxxAbiData, GuestError> {
    let mut abi = CxxAbiData::default();
    let pointer = ctx.pointer_bytes as i64;
    let class_name = "N10__cxxabiv117__class_type_infoE";
    let single_name = "N10__cxxabiv120__si_class_type_infoE";
    let multiple_name = "N10__cxxabiv121__vmi_class_type_infoE";
    let base = abi_data(ctx, memory, "_ZTISt9type_info", 2 * pointer as usize, "GLIBCXX_3.4")?;
    let class_info = abi_data(ctx, memory, &format!("_ZTI{class_name}"), 3 * pointer as usize, "CXXABI_1.3")?;
    let single_info = abi_data(ctx, memory, &format!("_ZTI{single_name}"), 3 * pointer as usize, "CXXABI_1.3")?;
    let multiple_info = abi_data(ctx, memory, &format!("_ZTI{multiple_name}"), 3 * pointer as usize, "CXXABI_1.3")?;
    let pointer_test = abi_function(
        ctx,
        memory,
        "__guest_type_info_is_pointer",
        &[GuestStorage::Pointer],
        Some(GuestStorage::Int32),
        Rc::new(|_, _, _| Ok(GuestCallResult::Value(GuestCallValue::Int32(0)))),
        "GLIBCXX_3.4",
    )?;
    let function_test = abi_function(
        ctx,
        memory,
        "__guest_type_info_is_function",
        &[GuestStorage::Pointer],
        Some(GuestStorage::Int32),
        Rc::new(|_, _, _| Ok(GuestCallResult::Value(GuestCallValue::Int32(0)))),
        "GLIBCXX_3.4",
    )?;
    for (form, name, info) in [
        ("class", class_name, class_info),
        ("si", single_name, single_info),
        ("vmi", multiple_name, multiple_info),
    ] {
        let table = abi_data(ctx, memory, &format!("_ZTV{name}"), 11 * pointer as usize, "CXXABI_1.3")?;
        memory.write_pointer(abi_slot(memory, table, pointer)?, Some(info))?;
        for index in 0..9 {
            let method = if index == 2 {
                pointer_test
            } else if index == 3 {
                function_test
            } else {
                abi_unsupported(ctx, memory, &format!("__guest_{form}_type_info_virtual_{index}"))?
            };
            memory.write_pointer(abi_slot(memory, table, (index + 2) * pointer)?, Some(method))?;
        }
        abi.class_info_vtables.insert(form.to_string(), abi_slot(memory, table, 2 * pointer)?);
    }
    let class_vtable = abi.class_info_vtables.get("class").copied();
    let single_vtable = abi.class_info_vtables.get("si").copied();
    let (Some(class_vtable), Some(single_vtable)) = (class_vtable, single_vtable) else {
        return Err(GuestError::callback("Missing RTTI class tables"));
    };
    memory.write_pointer(base, Some(class_vtable))?;
    let type_name = abi_bytes(ctx, memory, "St9type_info")?;
    memory.write_pointer(abi_slot(memory, base, pointer)?, Some(type_name))?;
    for (name, info, parent) in [
        (class_name, class_info, base),
        (single_name, single_info, class_info),
        (multiple_name, multiple_info, class_info),
    ] {
        memory.write_pointer(info, Some(single_vtable))?;
        let bytes = abi_bytes(ctx, memory, name)?;
        memory.write_pointer(abi_slot(memory, info, pointer)?, Some(bytes))?;
        memory.write_pointer(abi_slot(memory, info, 2 * pointer)?, Some(parent))?;
        abi.types.insert(name.to_string(), info);
    }
    abi.types.insert("St9type_info".to_string(), base);
    Ok(abi)
}

/// Concrete class RTTI, including encoded nonvirtual or virtual base offsets.
pub fn abi_type(
    ctx: &SystemVContext,
    memory: &mut SparseGuestMemory,
    abi: &SharedAbi,
    name: &str,
    bases: &[CxxBase],
) -> Result<GuestAddress, GuestError> {
    if let Some(prior) = abi.borrow().types.get(name).copied() {
        return Ok(prior);
    }
    let pointer = ctx.pointer_bytes as i64;
    let form = if bases.is_empty() {
        "class"
    } else if bases.len() == 1 && bases[0].offset == 0 && bases[0].flags == 2 {
        "si"
    } else {
        "vmi"
    };
    let vtable = abi
        .borrow()
        .class_info_vtables
        .get(form)
        .copied()
        .ok_or_else(|| GuestError::callback("Missing RTTI class table"))?;
    let size = if form == "class" {
        2 * pointer as usize
    } else if form == "si" {
        3 * pointer as usize
    } else {
        (2 * pointer + 8) as usize + bases.len() * 2 * pointer as usize
    };
    let address = abi_data(ctx, memory, &format!("_ZTI{name}"), size, "GLIBCXX_3.4")?;
    let text = abi_bytes(ctx, memory, name)?;
    memory.write_pointer(address, Some(vtable))?;
    memory.write_pointer(abi_slot(memory, address, pointer)?, Some(text))?;
    if form == "si" {
        let base = bases
            .first()
            .ok_or_else(|| GuestError::callback("Missing single RTTI base"))?;
        memory.write_pointer(abi_slot(memory, address, 2 * pointer)?, Some(base.address))?;
    } else if form == "vmi" {
        memory.write_u32(abi_slot(memory, address, 2 * pointer)?, 0)?;
        memory.write_u32(abi_slot(memory, address, 2 * pointer + 4)?, bases.len() as u32)?;
        for (index, base) in bases.iter().enumerate() {
            memory.write_pointer(
                abi_slot(memory, address, 2 * pointer + 8 + index as i64 * 2 * pointer)?,
                Some(base.address),
            )?;
            write_unsigned(
                memory,
                abi_slot(memory, address, 3 * pointer + 8 + index as i64 * 2 * pointer)?,
                ctx.pointer_bytes,
                i128::from(base.offset) * 256 + i128::from(base.flags),
            )?;
        }
    }
    abi.borrow_mut().types.insert(name.to_string(), address);
    Ok(address)
}

/// Build a vtable with `entries` after the RTTI slot; returns the address point.
pub fn abi_vtable(
    ctx: &SystemVContext,
    memory: &mut SparseGuestMemory,
    name: &str,
    info: GuestAddress,
    entries: &[GuestAddress],
) -> Result<GuestAddress, GuestError> {
    let pointer = ctx.pointer_bytes as i64;
    let table = abi_data(
        ctx,
        memory,
        &format!("_ZTV{name}"),
        (entries.len() + 2) * pointer as usize,
        "GLIBCXX_3.4",
    )?;
    memory.write_pointer(abi_slot(memory, table, pointer)?, Some(info))?;
    for (index, entry) in entries.iter().enumerate() {
        memory.write_pointer(
            abi_slot(memory, table, (index as i64 + 2) * pointer)?,
            Some(*entry),
        )?;
    }
    abi_slot(memory, table, 2 * pointer)
}
