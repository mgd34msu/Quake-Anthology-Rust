//! MSVC stream services: streambuf, ios, and ostream insertion.
//!
//! Donor: `src/guest/runtime/windows/msvc/streams.ts` (Microsoft STL
//! streambuf/xiosbase/ios, MS x64 ABI).

use std::rc::Rc;

use crate::core::callbacks::HostCallContext;
use crate::core::contracts::{
    GuestAddress, GuestAllocationOptions, GuestCallContext, GuestCallResult, GuestCallValue,
    GuestPermissions, GuestStorage,
};
use crate::core::memory::SparseGuestMemory;
use crate::error::GuestError;
use crate::runtime::common::memory::{integer, pointer, required_pointer};
use crate::runtime::windows::contracts::{
    invoke_nested, unsupported_windows, SharedWindows, WindowsContext, WindowsServiceRegistrar,
};
use crate::runtime::windows::msvc::locale::{install_msvc_locale, MsvcLocale};

const LIBRARY: &str = "msvcp140.dll";
const STREAMBUF: &str = "?$basic_streambuf@DU?$char_traits@D@std@@@std@@";
const IOS: &str = "?$basic_ios@DU?$char_traits@D@std@@@std@@";
const OSTREAM: &str = "?$basic_ostream@DU?$char_traits@D@std@@@std@@";
const IOSTREAM: &str = "?$basic_iostream@DU?$char_traits@D@std@@@std@@";

fn ptr_value(value: Option<GuestAddress>) -> GuestCallResult {
    GuestCallResult::Value(GuestCallValue::Pointer(value))
}

fn i32_value(value: i32) -> GuestCallResult {
    GuestCallResult::Value(GuestCallValue::Int32(value))
}

fn result_integer(result: GuestCallResult) -> Result<i128, GuestError> {
    match result {
        GuestCallResult::Value(GuestCallValue::Int32(value)) => Ok(i128::from(value)),
        GuestCallResult::Value(GuestCallValue::Uint32(value)) => Ok(i128::from(value)),
        GuestCallResult::Value(GuestCallValue::Int64(value)) => Ok(i128::from(value)),
        GuestCallResult::Value(GuestCallValue::Uint64(value)) => Ok(i128::from(value)),
        _ => Err(GuestError::invalid("MSVC integer result required")),
    }
}

fn uint_to_radix(mut value: u64, radix: u32) -> String {
    if value == 0 {
        return "0".to_string();
    }
    let mut digits = Vec::new();
    while value > 0 {
        digits.push(char::from_digit((value % u64::from(radix)) as u32, radix).expect("radix digit"));
        value /= u64::from(radix);
    }
    digits.iter().rev().collect()
}

fn ref_pointer(memory: &mut SparseGuestMemory, address: GuestAddress) -> Result<GuestAddress, GuestError> {
    memory
        .read_pointer(address)?
        .ok_or_else(|| GuestError::callback("Missing MSVC object pointer"))
}

/// Constructed MSVC stream tables and locale.
#[derive(Debug, Clone)]
pub struct MsvcStreams {
    locale: MsvcLocale,
    context: WindowsContext,
    shared: SharedWindows,
    width: usize,
    ios_table: GuestAddress,
    ostream_table: GuestAddress,
    iostream_table: GuestAddress,
    buffer_table: GuestAddress,
    ostream_vb: GuestAddress,
    iostream_vb: GuestAddress,
    teb: GuestAddress,
}

impl MsvcStreams {
    fn virtual_call(
        &self,
        ctx: &mut HostCallContext<'_, '_>,
        context: &GuestCallContext,
        object: GuestAddress,
        slot: usize,
        parameters: &[GuestStorage],
        result: Option<GuestStorage>,
        args: Vec<GuestCallValue>,
    ) -> Result<GuestCallResult, GuestError> {
        let memory = ctx.memory();
        let vtable = ref_pointer(memory, object)?;
        let target = ref_pointer(memory, memory.offset(vtable, (slot * 8) as i64)?)?;
        let mut full_parameters = vec![GuestStorage::Pointer];
        full_parameters.extend_from_slice(parameters);
        let mut full_args = vec![GuestCallValue::Pointer(Some(object))];
        full_args.extend(args);
        invoke_nested(ctx, &self.shared, self.width, context, target, &full_parameters, result, full_args, false)
    }

    fn virtual_ios(&self, memory: &mut SparseGuestMemory, object: GuestAddress) -> Result<GuestAddress, GuestError> {
        let vtable = ref_pointer(memory, object)?;
        let displacement = memory.read_i32(memory.offset(vtable, 4)?)?;
        memory.offset(object, i64::from(displacement))
    }

    fn setstate(
        &self,
        memory: &mut SparseGuestMemory,
        object: GuestAddress,
        state: i32,
    ) -> Result<(), GuestError> {
        let value = (memory.read_i32(memory.offset(object, 16)?)?
            | state
            | if memory.read_pointer(memory.offset(object, 72)?)?.is_none() {
                4
            } else {
                0
            })
            & 0x17;
        memory.write_i32(memory.offset(object, 16)?, value)?;
        if value & memory.read_i32(memory.offset(object, 20)?)? != 0 {
            return Err(unsupported_windows(
                LIBRARY,
                "ios_base::failure",
                "stream exception requires guest C++ exception dispatch",
            ));
        }
        Ok(())
    }

    fn basic_dtor(
        &self,
        ctx: &mut HostCallContext<'_, '_>,
        context: &GuestCallContext,
        object: GuestAddress,
    ) -> Result<(), GuestError> {
        let memory = ctx.memory();
        memory.write_pointer(object, Some(self.ios_table))?;
        if memory.read_u64(memory.offset(object, 8)?)? != 0 {
            return Err(unsupported_windows(
                LIBRARY,
                "ios_base destructor",
                "standard stream ownership is not implemented",
            ));
        }
        // _Callfns(erase_event), then destroy the sparse iword/pword and event lists.
        let mut call = memory.read_pointer(memory.offset(object, 56)?)?;
        while let Some(entry) = call {
            let memory = ctx.memory();
            let target = ref_pointer(memory, memory.offset(entry, 16)?)?;
            let index = memory.read_i32(memory.offset(entry, 8)?)?;
            invoke_nested(
                ctx,
                &self.shared,
                self.width,
                context,
                target,
                &[GuestStorage::Int32, GuestStorage::Pointer, GuestStorage::Int32],
                None,
                vec![
                    GuestCallValue::Int32(0),
                    GuestCallValue::Pointer(Some(object)),
                    GuestCallValue::Int32(index),
                ],
                false,
            )?;
            call = ctx.memory().read_pointer(entry)?;
        }
        for offset in [48, 56] {
            let mut node = ctx.memory().read_pointer(ctx.memory().offset(object, offset)?)?;
            while let Some(entry) = node {
                let next = ctx.memory().read_pointer(entry)?;
                if !self.context.free(ctx.memory(), self.teb, Some(entry), 0)? {
                    return Err(GuestError::callback("Invalid ios_base list allocation"));
                }
                node = next;
            }
            let memory = ctx.memory();
            memory.write_pointer(memory.offset(object, offset)?, None)?;
        }
        let memory = ctx.memory();
        let implementation = memory.read_pointer(memory.offset(object, 64)?)?;
        self.locale.destroy(memory, implementation)?;
        memory.write_pointer(memory.offset(object, 64)?, None)?;
        Ok(())
    }

    fn initialize(
        &self,
        memory: &mut SparseGuestMemory,
        object: GuestAddress,
        buffer: Option<GuestAddress>,
    ) -> Result<(), GuestError> {
        memory.write_u64(memory.offset(object, 8)?, 0)?;
        memory.write_i32(memory.offset(object, 16)?, if buffer.is_none() { 4 } else { 0 })?;
        memory.write_i32(memory.offset(object, 20)?, 0)?;
        memory.write_i32(memory.offset(object, 24)?, 0x201)?;
        memory.write_i64(memory.offset(object, 32)?, 6)?;
        memory.write_i64(memory.offset(object, 40)?, 0)?;
        memory.write_pointer(memory.offset(object, 48)?, None)?;
        memory.write_pointer(memory.offset(object, 56)?, None)?;
        let implementation = self.locale.create(memory)?;
        memory.write_pointer(memory.offset(object, 64)?, Some(implementation))?;
        memory.write_pointer(memory.offset(object, 72)?, buffer)?;
        memory.write_pointer(memory.offset(object, 80)?, None)?;
        memory.write_u8(memory.offset(object, 88)?, 32)?;
        Ok(())
    }

    fn field(
        &self,
        memory: &mut SparseGuestMemory,
        object: GuestAddress,
        offset: i64,
    ) -> Result<Option<GuestAddress>, GuestError> {
        memory.read_pointer(ref_pointer(memory, memory.offset(object, offset)?)?)
    }

    fn available(
        &self,
        memory: &mut SparseGuestMemory,
        object: GuestAddress,
        input: bool,
    ) -> Result<i64, GuestError> {
        if self.field(memory, object, if input { 56 } else { 64 })?.is_none() {
            return Ok(0);
        }
        let slot = ref_pointer(memory, memory.offset(object, if input { 80 } else { 88 })?)?;
        Ok(i64::from(memory.read_i32(slot)?))
    }

    fn bump(
        &self,
        memory: &mut SparseGuestMemory,
        object: GuestAddress,
        amount: i64,
        input: bool,
    ) -> Result<GuestAddress, GuestError> {
        let next_slot = ref_pointer(memory, memory.offset(object, if input { 56 } else { 64 })?)?;
        let next = ref_pointer(memory, next_slot)?;
        let count_slot = ref_pointer(memory, memory.offset(object, if input { 80 } else { 88 })?)?;
        memory.write_pointer(next_slot, Some(memory.offset(next, amount)?))?;
        memory.write_i32(count_slot, memory.read_i32(count_slot)? - amount as i32)?;
        Ok(next)
    }

    fn stream_get(
        &self,
        ctx: &mut HostCallContext<'_, '_>,
        context: &GuestCallContext,
        object: GuestAddress,
        consume: bool,
    ) -> Result<i128, GuestError> {
        if self.available(ctx.memory(), object, true)? > 0 {
            let memory = ctx.memory();
            let address = if consume {
                self.bump(memory, object, 1, true)?
            } else {
                ref_pointer(memory, ref_pointer(memory, memory.offset(object, 56)?)?)?
            };
            return Ok(i128::from(memory.read_u8(address)?));
        }
        let result = self.virtual_call(ctx, context, object, if consume { 7 } else { 6 }, &[], Some(GuestStorage::Int32), vec![])?;
        result_integer(result)
    }

    fn stream_put(
        &self,
        ctx: &mut HostCallContext<'_, '_>,
        context: &GuestCallContext,
        object: GuestAddress,
        character: i128,
    ) -> Result<i128, GuestError> {
        if self.available(ctx.memory(), object, false)? > 0 {
            let memory = ctx.memory();
            let slot = self.bump(memory, object, 1, false)?;
            memory.write_u8(slot, (character & 255) as u8)?;
            return Ok(character & 255);
        }
        let result = self.virtual_call(
            ctx,
            context,
            object,
            3,
            &[GuestStorage::Int32],
            Some(GuestStorage::Int32),
            vec![GuestCallValue::Int32((character & 255) as i32)],
        )?;
        result_integer(result)
    }

    fn flush(
        &self,
        ctx: &mut HostCallContext<'_, '_>,
        context: &GuestCallContext,
        object: GuestAddress,
    ) -> Result<(), GuestError> {
        let memory = ctx.memory();
        let base = self.virtual_ios(memory, object)?;
        let buffer = memory.read_pointer(memory.offset(base, 72)?)?;
        let Some(buffer) = buffer else {
            return Ok(());
        };
        self.virtual_call(ctx, context, buffer, 1, &[], None, vec![])?;
        let inner = self.flush_inner(ctx, context, object, base, buffer);
        let unlock = self.virtual_call(ctx, context, buffer, 2, &[], None, vec![]);
        match (inner, unlock) {
            (Err(_), Err(unlock)) => Err(unlock),
            (Err(inner), Ok(())) => Err(inner),
            (Ok(()), result) => result,
        }
    }

    fn flush_inner(
        &self,
        ctx: &mut HostCallContext<'_, '_>,
        context: &GuestCallContext,
        object: GuestAddress,
        base: GuestAddress,
        buffer: GuestAddress,
    ) -> Result<(), GuestError> {
        if ctx.memory().read_i32(ctx.memory().offset(base, 16)?)? == 0 {
            let tied = ctx.memory().read_pointer(ctx.memory().offset(base, 80)?)?;
            if let Some(tied) = tied {
                if tied.offset != object.offset {
                    self.flush(ctx, context, tied)?;
                }
            }
            let memory = ctx.memory();
            if memory.read_i32(memory.offset(base, 16)?)? == 0 {
                let synced = self.virtual_call(ctx, context, buffer, 13, &[], Some(GuestStorage::Int32), vec![])?;
                if result_integer(synced)? == -1 {
                    self.setstate(ctx.memory(), base, 4)?;
                }
            }
        }
        self.suffix(ctx, context, object)
    }

    fn suffix(
        &self,
        ctx: &mut HostCallContext<'_, '_>,
        context: &GuestCallContext,
        object: GuestAddress,
    ) -> Result<(), GuestError> {
        let memory = ctx.memory();
        let base = self.virtual_ios(memory, object)?;
        if memory.read_i32(memory.offset(base, 16)?)? != 0
            || memory.read_i32(memory.offset(base, 24)?)? & 2 == 0
        {
            return Ok(());
        }
        let buffer = memory.read_pointer(memory.offset(base, 72)?)?;
        if let Some(buffer) = buffer {
            let synced = self.virtual_call(ctx, context, buffer, 13, &[], Some(GuestStorage::Int32), vec![])?;
            if result_integer(synced)? == -1 {
                self.setstate(ctx.memory(), base, 4)?;
            }
        }
        Ok(())
    }
}

fn direct(memory: &mut SparseGuestMemory, byte_length: usize, label: &str) -> Result<GuestAddress, GuestError> {
    memory.allocate(&GuestAllocationOptions {
        byte_length,
        alignment: 16,
        permissions: GuestPermissions::ReadWrite,
        label: label.to_string(),
    })
}

fn build_table(
    context: &WindowsContext,
    memory: &mut SparseGuestMemory,
    names: &[String],
    label: &str,
) -> Result<GuestAddress, GuestError> {
    let result = direct(memory, names.len() * 8, label)?;
    for (index, name) in names.iter().enumerate() {
        let target = context
            .resolve_address(LIBRARY, name)
            .ok_or_else(|| GuestError::callback(format!("Missing MSVC method {name}")))?;
        memory.write_pointer(memory.offset(result, (index * 8) as i64)?, Some(target))?;
    }
    memory.protect(result, names.len() * 8, GuestPermissions::Read)?;
    Ok(result)
}


fn direct(memory: &mut SparseGuestMemory, byte_length: usize, label: &str) -> Result<GuestAddress, GuestError> {
    memory.allocate(&GuestAllocationOptions {
        byte_length,
        alignment: 16,
        permissions: GuestPermissions::ReadWrite,
        label: label.to_string(),
    })
}

fn fill_table(
    context: &WindowsContext,
    memory: &mut SparseGuestMemory,
    table: GuestAddress,
    names: &[String],
) -> Result<(), GuestError> {
    for (index, name) in names.iter().enumerate() {
        let target = context
            .resolve_address(LIBRARY, name)
            .ok_or_else(|| GuestError::callback(format!("Missing MSVC method {name}")))?;
        memory.write_pointer(memory.offset(table, (index * 8) as i64)?, Some(target))?;
    }
    memory.protect(table, names.len() * 8, GuestPermissions::Read)?;
    Ok(())
}

/// Register the MSVC stream service set (x64 only).
pub fn install_msvc_streams(host: &mut WindowsServiceRegistrar<'_>) -> Result<(), GuestError> {
    if host.pointer_bytes != 8 {
        return Ok(());
    }
    let locale = install_msvc_locale(host)?;
    let context = host.context.clone();
    let shared = Rc::clone(&host.shared);
    let width = host.pointer_bytes;
    let teb = host.teb;
    // Table addresses are final up front; filling below runs before any
    // registered closure can execute.
    let ios_table = direct(host.memory, 8, "MSVC basic_ios vftable")?;
    let ostream_table = direct(host.memory, 8, "MSVC ostream vftable")?;
    let iostream_table = direct(host.memory, 8, "MSVC iostream vftable")?;
    let buffer_table = direct(host.memory, 15 * 8, "MSVC streambuf vftable")?;
    let ostream_vb = direct(host.memory, 8, "MSVC ostream vbtable")?;
    host.memory.write_i32(host.memory.offset(ostream_vb, 4)?, 16)?;
    host.memory.protect(ostream_vb, 8, GuestPermissions::Read)?;
    let iostream_vb = direct(host.memory, 16, "MSVC iostream vbtables")?;
    host.memory.write_i32(host.memory.offset(iostream_vb, 4)?, 32)?;
    host.memory.write_i32(host.memory.offset(iostream_vb, 12)?, 16)?;
    host.memory.protect(iostream_vb, 16, GuestPermissions::Read)?;
    let streams = MsvcStreams {
        locale,
        context: context.clone(),
        shared,
        width,
        ios_table,
        ostream_table,
        iostream_table,
        buffer_table,
        ostream_vb,
        iostream_vb,
        teb,
    };
    install_ios(host, &streams)?;
    fill_table(&context, host.memory, ios_table, &["runtime:basic-ios-delete".to_string()])?;
    install_ostream(host, &streams)?;
    fill_table(&context, host.memory, ostream_table, &["runtime:ostream-delete".to_string()])?;
    fill_table(&context, host.memory, iostream_table, &["runtime:ostream-delete".to_string()])?;
    install_streambuf(host, &streams)?;
    fill_table(
        &context,
        host.memory,
        buffer_table,
        &[
            "runtime:streambuf-delete".to_string(),
            format!("?_Lock@{STREAMBUF}UEAAXXZ"),
            format!("?_Unlock@{STREAMBUF}UEAAXXZ"),
            "runtime:streambuf-overflow".to_string(),
            "runtime:streambuf-pbackfail".to_string(),
            format!("?showmanyc@{STREAMBUF}MEAA_JXZ"),
            "runtime:streambuf-underflow".to_string(),
            format!("?uflow@{STREAMBUF}MEAAHXZ"),
            format!("?xsgetn@{STREAMBUF}MEAA_JPEAD_J@Z"),
            format!("?xsputn@{STREAMBUF}MEAA_JPEBD_J@Z"),
            "runtime:streambuf-seekoff".to_string(),
            "runtime:streambuf-seekpos".to_string(),
            format!("?setbuf@{STREAMBUF}MEAAPEAV12@PEAD_J@Z"),
            format!("?sync@{STREAMBUF}MEAAHXZ"),
            format!("?imbue@{STREAMBUF}MEAAXAEBVlocale@2@@Z"),
        ],
    )?;
    {
        let streams = streams.clone();
        host.service(LIBRARY, &format!("??0{STREAMBUF}IEAA@XZ"), &[GuestStorage::Pointer], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let object = required_pointer(args, 0)?;
                let memory = ctx.memory();
                memory.write(object, &[0u8; 104])?;
                memory.write_pointer(object, Some(streams.buffer_table))?;
                for (slot, target) in [(24, 8), (32, 16), (56, 40), (64, 48), (80, 72), (88, 76)] {
                    memory.write_pointer(memory.offset(object, slot)?, Some(memory.offset(object, target)?))?;
                }
                let implementation = streams.locale.create(memory)?;
                memory.write_pointer(memory.offset(object, 96)?, Some(implementation))?;
                Ok(ptr_value(Some(object)))
            },
        ))?;
    }
    install_insertions(host, &streams)?;
    Ok(())
}

fn install_ios(host: &mut WindowsServiceRegistrar<'_>, streams: &MsvcStreams) -> Result<(), GuestError> {
    let teb = streams.teb;
    {
        let streams = streams.clone();
        host.service(LIBRARY, &format!("??1{IOS}UEAA@XZ"), &[GuestStorage::Pointer], None, Rc::new(
            move |ctx, context, args| {
                streams.basic_dtor(ctx, context, required_pointer(args, 0)?)?;
                Ok(GuestCallResult::Void)
            },
        ))?;
    }
    {
        let streams = streams.clone();
        host.service(LIBRARY, "runtime:basic-ios-delete", &[GuestStorage::Pointer, GuestStorage::Uint32], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, context, args| {
                let object = required_pointer(args, 0)?;
                streams.basic_dtor(ctx, context, object)?;
                if integer(args, 1)? & 1 != 0 && !streams.context.free(ctx.memory(), teb, Some(object), 0)? {
                    return Err(GuestError::callback("Invalid basic_ios allocation"));
                }
                Ok(ptr_value(Some(object)))
            },
        ))?;
    }
    {
        let streams = streams.clone();
        host.service(LIBRARY, &format!("??0{IOS}IEAA@XZ"), &[GuestStorage::Pointer], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let object = required_pointer(args, 0)?;
                let memory = ctx.memory();
                memory.write(object, &[0u8; 96])?;
                memory.write_pointer(object, Some(streams.ios_table))?;
                Ok(ptr_value(Some(object)))
            },
        ))?;
    }
    host.service(LIBRARY, &format!("?rdbuf@{IOS}QEBAPEAV?$basic_streambuf@DU?$char_traits@D@std@@@2@XZ"), &[GuestStorage::Pointer], Some(GuestStorage::Pointer), Rc::new(
        move |ctx, _, args| {
            let memory = ctx.memory();
            Ok(ptr_value(memory.read_pointer(memory.offset(required_pointer(args, 0)?, 72)?)?))
        },
    ))?;
    {
        let streams = streams.clone();
        host.service(LIBRARY, &format!("?setstate@{IOS}QEAAXH_N@Z"), &[GuestStorage::Pointer, GuestStorage::Int32, GuestStorage::Uint32], None, Rc::new(
            move |ctx, _, args| {
                streams.setstate(ctx.memory(), required_pointer(args, 0)?, integer(args, 1)? as i32)?;
                Ok(GuestCallResult::Void)
            },
        ))?;
    }
    host.service(LIBRARY, "?good@ios_base@std@@QEBA_NXZ", &[GuestStorage::Pointer], Some(GuestStorage::Uint32), Rc::new(
        move |ctx, _, args| {
            let memory = ctx.memory();
            let good = memory.read_i32(memory.offset(required_pointer(args, 0)?, 16)?)? == 0;
            Ok(GuestCallResult::Value(GuestCallValue::Uint32(u32::from(good))))
        },
    ))?;
    Ok(())
}

fn install_ostream(host: &mut WindowsServiceRegistrar<'_>, streams: &MsvcStreams) -> Result<(), GuestError> {
    {
        let streams = streams.clone();
        host.service(LIBRARY, &format!("??0{OSTREAM}QEAA@PEAV?$basic_streambuf@DU?$char_traits@D@std@@@1@_N@Z"), &[GuestStorage::Pointer, GuestStorage::Pointer, GuestStorage::Uint32, GuestStorage::Int32], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let object = required_pointer(args, 0)?;
                if integer(args, 2)? != 0 {
                    return Err(unsupported_windows(
                        LIBRARY,
                        "basic_ostream constructor",
                        "standard stream registration is not implemented",
                    ));
                }
                let memory = ctx.memory();
                if integer(args, 3)? != 0 {
                    memory.write_pointer(object, Some(streams.ostream_vb))?;
                    let ios = memory.offset(object, 16)?;
                    memory.write(ios, &[0u8; 96])?;
                    memory.write_pointer(ios, Some(streams.ios_table))?;
                }
                let base = streams.virtual_ios(memory, object)?;
                memory.write_pointer(base, Some(streams.ostream_table))?;
                memory.write_i32(memory.offset(base, -4)?, (base.offset - object.offset) as i32 - 16)?;
                streams.initialize(memory, base, pointer(args, 1)?)?;
                Ok(ptr_value(Some(object)))
            },
        ))?;
    }
    {
        let streams = streams.clone();
        host.service(LIBRARY, &format!("??0{IOSTREAM}QEAA@PEAV?$basic_streambuf@DU?$char_traits@D@std@@@1@@Z"), &[GuestStorage::Pointer, GuestStorage::Pointer, GuestStorage::Int32], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let object = required_pointer(args, 0)?;
                let memory = ctx.memory();
                if integer(args, 2)? != 0 {
                    memory.write_pointer(object, Some(streams.iostream_vb))?;
                    memory.write_pointer(memory.offset(object, 16)?, Some(memory.offset(streams.iostream_vb, 8)?))?;
                    let ios = memory.offset(object, 32)?;
                    memory.write(ios, &[0u8; 96])?;
                    memory.write_pointer(ios, Some(streams.ios_table))?;
                }
                memory.write_i64(memory.offset(object, 8)?, 0)?;
                let base = streams.virtual_ios(memory, object)?;
                memory.write_pointer(base, Some(streams.iostream_table))?;
                memory.write_i32(memory.offset(base, -4)?, (base.offset - object.offset) as i32 - 32)?;
                streams.initialize(memory, base, pointer(args, 1)?)?;
                Ok(ptr_value(Some(object)))
            },
        ))?;
    }
    // MSVC base destructors receive this adjusted to the vfptr's static offset,
    // without the dynamic virtual-base displacement (retail call RVA 0x80f9c).
    // basic_ios is destroyed separately by the most-derived destructor.
    {
        let streams = streams.clone();
        host.service(LIBRARY, &format!("??1{OSTREAM}UEAA@XZ"), &[GuestStorage::Pointer], None, Rc::new(
            move |ctx, _, args| {
                let memory = ctx.memory();
                let object = memory.offset(required_pointer(args, 0)?, -16)?;
                let base = streams.virtual_ios(memory, object)?;
                memory.write_pointer(base, Some(streams.ostream_table))?;
                memory.write_i32(memory.offset(base, -4)?, (base.offset - object.offset) as i32 - 16)?;
                Ok(GuestCallResult::Void)
            },
        ))?;
    }
    {
        let streams = streams.clone();
        host.service(LIBRARY, &format!("??1{IOSTREAM}UEAA@XZ"), &[GuestStorage::Pointer], None, Rc::new(
            move |ctx, _, args| {
                let memory = ctx.memory();
                let object = memory.offset(required_pointer(args, 0)?, -32)?;
                let base = streams.virtual_ios(memory, object)?;
                memory.write_pointer(base, Some(streams.iostream_table))?;
                memory.write_i32(memory.offset(base, -4)?, (base.offset - object.offset) as i32 - 32)?;
                Ok(GuestCallResult::Void)
            },
        ))?;
    }
    host.service(LIBRARY, "runtime:ostream-delete", &[GuestStorage::Pointer, GuestStorage::Uint32], Some(GuestStorage::Pointer), Rc::new(
        move |_, _, args| {
            Err(unsupported_windows(
                LIBRARY,
                "basic_ostream deleting destructor",
                format!(
                    "standalone deleting destructor at {:x} is not implemented",
                    required_pointer(args, 0)?.offset
                ),
            ))
        },
    ))?;
    Ok(())
}

fn install_streambuf(host: &mut WindowsServiceRegistrar<'_>, streams: &MsvcStreams) -> Result<(), GuestError> {
    let teb = streams.teb;
    {
        let streams = streams.clone();
        host.service(LIBRARY, &format!("??1{STREAMBUF}UEAA@XZ"), &[GuestStorage::Pointer], None, Rc::new(
            move |ctx, _, args| {
                let object = required_pointer(args, 0)?;
                let memory = ctx.memory();
                memory.write_pointer(object, Some(streams.buffer_table))?;
                let implementation = memory.read_pointer(memory.offset(object, 96)?)?;
                streams.locale.destroy(memory, implementation)?;
                memory.write_pointer(memory.offset(object, 96)?, None)?;
                Ok(GuestCallResult::Void)
            },
        ))?;
    }
    {
        let streams = streams.clone();
        host.service(LIBRARY, "runtime:streambuf-delete", &[GuestStorage::Pointer, GuestStorage::Uint32], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let object = required_pointer(args, 0)?;
                let memory = ctx.memory();
                memory.write_pointer(object, Some(streams.buffer_table))?;
                let implementation = memory.read_pointer(memory.offset(object, 96)?)?;
                streams.locale.destroy(memory, implementation)?;
                memory.write_pointer(memory.offset(object, 96)?, None)?;
                if integer(args, 1)? & 1 != 0 && !streams.context.free(memory, teb, Some(object), 0)? {
                    return Err(GuestError::callback("Invalid streambuf allocation"));
                }
                Ok(ptr_value(Some(object)))
            },
        ))?;
    }
    // Base streambuf has no external lock, device, seek operation or imbue action.
    for method in ["_Lock", "_Unlock"] {
        host.service(LIBRARY, &format!("?{method}@{STREAMBUF}UEAAXXZ"), &[GuestStorage::Pointer], None, Rc::new(
            move |_, _, _| Ok(GuestCallResult::Void),
        ))?;
    }
    for method in ["overflow", "pbackfail"] {
        host.service(LIBRARY, &format!("runtime:streambuf-{method}"), &[GuestStorage::Pointer, GuestStorage::Int32], Some(GuestStorage::Int32), Rc::new(
            move |_, _, _| Ok(i32_value(-1)),
        ))?;
    }
    host.service(LIBRARY, "runtime:streambuf-underflow", &[GuestStorage::Pointer], Some(GuestStorage::Int32), Rc::new(
        move |_, _, _| Ok(i32_value(-1)),
    ))?;
    host.service(LIBRARY, &format!("?showmanyc@{STREAMBUF}MEAA_JXZ"), &[GuestStorage::Pointer], Some(GuestStorage::Int64), Rc::new(
        move |_, _, _| Ok(GuestCallResult::Value(GuestCallValue::Int64(0))),
    ))?;
    host.service(LIBRARY, &format!("?sync@{STREAMBUF}MEAAHXZ"), &[GuestStorage::Pointer], Some(GuestStorage::Int32), Rc::new(
        move |_, _, _| Ok(i32_value(0)),
    ))?;
    host.service(LIBRARY, &format!("?setbuf@{STREAMBUF}MEAAPEAV12@PEAD_J@Z"), &[GuestStorage::Pointer, GuestStorage::Pointer, GuestStorage::Int64], Some(GuestStorage::Pointer), Rc::new(
        move |_, _, args| Ok(ptr_value(Some(required_pointer(args, 0)?))),
    ))?;
    host.service(LIBRARY, &format!("?imbue@{STREAMBUF}MEAAXAEBVlocale@2@@Z"), &[GuestStorage::Pointer, GuestStorage::Pointer], None, Rc::new(
        move |_, _, _| Ok(GuestCallResult::Void),
    ))?;
    for (name, offset) in [("eback", 24), ("pbase", 32), ("gptr", 56), ("pptr", 64)] {
        let streams = streams.clone();
        host.service(LIBRARY, &format!("?{name}@{STREAMBUF}IEBAPEADXZ"), &[GuestStorage::Pointer], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                Ok(ptr_value(streams.field(ctx.memory(), required_pointer(args, 0)?, offset)?))
            },
        ))?;
    }
    for input in [true, false] {
        let streams = streams.clone();
        let name = if input { "egptr" } else { "epptr" };
        host.service(LIBRARY, &format!("?{name}@{STREAMBUF}IEBAPEADXZ"), &[GuestStorage::Pointer], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let object = required_pointer(args, 0)?;
                let memory = ctx.memory();
                let next = streams.field(memory, object, if input { 56 } else { 64 })?;
                let Some(next) = next else {
                    return Ok(ptr_value(None));
                };
                let count = ref_pointer(memory, memory.offset(object, if input { 80 } else { 88 })?)?;
                let end = memory.offset(next, i64::from(memory.read_i32(count)?))?;
                Ok(ptr_value(Some(end)))
            },
        ))?;
    }
    {
        let streams = streams.clone();
        host.service(LIBRARY, &format!("?uflow@{STREAMBUF}MEAAHXZ"), &[GuestStorage::Pointer], Some(GuestStorage::Int32), Rc::new(
            move |ctx, context, args| {
                let object = required_pointer(args, 0)?;
                let underflow = streams.virtual_call(ctx, context, object, 6, &[], Some(GuestStorage::Int32), vec![])?;
                if result_integer(underflow)? == -1 {
                    return Ok(i32_value(-1));
                }
                let memory = ctx.memory();
                let slot = streams.bump(memory, object, 1, true)?;
                Ok(i32_value(i32::from(memory.read_u8(slot)?)))
            },
        ))?;
    }
    {
        let streams = streams.clone();
        host.service(LIBRARY, &format!("?sputc@{STREAMBUF}QEAAHD@Z"), &[GuestStorage::Pointer, GuestStorage::Int32], Some(GuestStorage::Int32), Rc::new(
            move |ctx, context, args| {
                let value = streams.stream_put(ctx, context, required_pointer(args, 0)?, integer(args, 1)?)?;
                Ok(i32_value(value as i32))
            },
        ))?;
    }
    for input in [true, false] {
        let streams = streams.clone();
        let name = if input {
            format!("?xsgetn@{STREAMBUF}MEAA_JPEAD_J@Z")
        } else {
            format!("?xsputn@{STREAMBUF}MEAA_JPEBD_J@Z")
        };
        host.service(LIBRARY, &name, &[GuestStorage::Pointer, GuestStorage::Pointer, GuestStorage::Int64], Some(GuestStorage::Int64), Rc::new(
            move |ctx, context, args| {
                let object = required_pointer(args, 0)?;
                let data = required_pointer(args, 1)?;
                let requested = integer(args, 2)?;
                if requested <= 0 {
                    return Ok(GuestCallResult::Value(GuestCallValue::Int64(0)));
                }
                if requested > 0x1000_0000 {
                    return Err(GuestError::invalid("MSVC stream transfer exceeds limit"));
                }
                let length = requested as usize;
                let mut copied = 0;
                while copied < length {
                    let amount = (streams.available(ctx.memory(), object, input)? as usize).min(length - copied);
                    if amount > 0 {
                        let memory = ctx.memory();
                        let buffer = streams.bump(memory, object, amount as i64, input)?;
                        if input {
                            let bytes = memory.copy(buffer, amount)?;
                            memory.write(memory.offset(data, copied as i64)?, &bytes)?;
                        } else {
                            let bytes = memory.copy(memory.offset(data, copied as i64)?, amount)?;
                            memory.write(buffer, &bytes)?;
                        }
                        copied += amount;
                    } else if input {
                        let character = streams.stream_get(ctx, context, object, true)?;
                        if character == -1 {
                            break;
                        }
                        let memory = ctx.memory();
                        memory.write_u8(memory.offset(data, copied as i64)?, (character & 255) as u8)?;
                        copied += 1;
                    } else {
                        let memory = ctx.memory();
                        let byte = memory.read_u8(memory.offset(data, copied as i64)?)?;
                        if streams.stream_put(ctx, context, object, i128::from(byte))? == -1 {
                            break;
                        }
                        copied += 1;
                    }
                }
                Ok(GuestCallResult::Value(GuestCallValue::Int64(copied as i64)))
            },
        ))?;
    }
    {
        let streams = streams.clone();
        host.service(LIBRARY, &format!("?sputn@{STREAMBUF}QEAA_JPEBD_J@Z"), &[GuestStorage::Pointer, GuestStorage::Pointer, GuestStorage::Int64], Some(GuestStorage::Int64), Rc::new(
            move |ctx, context, args| {
                streams.virtual_call(
                    ctx,
                    context,
                    required_pointer(args, 0)?,
                    9,
                    &[GuestStorage::Pointer, GuestStorage::Int64],
                    Some(GuestStorage::Int64),
                    vec![
                        GuestCallValue::Pointer(Some(required_pointer(args, 1)?)),
                        GuestCallValue::Int64(integer(args, 2)? as i64),
                    ],
                )
            },
        ))?;
    }
    for method in ["seekoff", "seekpos"] {
        let parameters: Vec<GuestStorage> = if method == "seekoff" {
            vec![GuestStorage::Pointer, GuestStorage::Pointer, GuestStorage::Int64, GuestStorage::Int32, GuestStorage::Int32]
        } else {
            vec![GuestStorage::Pointer, GuestStorage::Pointer, GuestStorage::Pointer, GuestStorage::Int32]
        };
        host.service(LIBRARY, &format!("runtime:streambuf-{method}"), &parameters, Some(GuestStorage::Pointer), Rc::new(
            move |ctx, _, args| {
                let memory = ctx.memory();
                let result = required_pointer(args, 1)?;
                memory.write(result, &[0u8; 24])?;
                memory.write_i64(memory.offset(result, 8)?, -1)?;
                Ok(ptr_value(Some(result)))
            },
        ))?;
    }
    Ok(())
}

impl MsvcStreams {
    fn flush(
        &self,
        host: &HostCallContext,
        nested: &WindowsContext,
        object: GuestAddress,
    ) -> Result<(), GuestError> {
        let buffer = {
            let memory = host.memory();
            let base = self.virtual_ios(memory, object)?;
            memory.read_pointer(memory.offset(base, 72)?)?
        };
        let Some(buffer) = buffer else {
            return Ok(());
        };
        self.virtual_call(host, nested, buffer, 1, &[], None, vec![])?;
        let outcome = self.flush_body(host, nested, object, buffer);
        let unlock = self.virtual_call(host, nested, buffer, 2, &[], None, vec![]);
        outcome.and(unlock.map(|_| ()))
    }

    fn flush_body(
        &self,
        host: &HostCallContext,
        nested: &WindowsContext,
        object: GuestAddress,
        buffer: GuestAddress,
    ) -> Result<(), GuestError> {
        let base = {
            let memory = host.memory();
            self.virtual_ios(memory, object)?
        };
        let state = {
            let memory = host.memory();
            memory.read_i32(memory.offset(base, 16)?)?
        };
        if state == 0 {
            let tied = {
                let memory = host.memory();
                memory.read_pointer(memory.offset(base, 80)?)?
            };
            if let Some(tied) = tied {
                if tied != object {
                    self.flush(host, nested, tied)?;
                }
            }
            let state = {
                let memory = host.memory();
                memory.read_i32(memory.offset(base, 16)?)?
            };
            if state == 0 {
                let synced = self.virtual_call(host, nested, buffer, 13, &[], Some(GuestStorage::Int32), vec![])?;
                if result_integer(synced)? == -1 {
                    self.setstate(host.memory(), base, 4)?;
                }
            }
        }
        self.suffix(host, nested, object)
    }

    fn suffix(
        &self,
        host: &HostCallContext,
        nested: &WindowsContext,
        object: GuestAddress,
    ) -> Result<(), GuestError> {
        let (base, state, flags) = {
            let memory = host.memory();
            let base = self.virtual_ios(memory, object)?;
            let state = memory.read_i32(memory.offset(base, 16)?)?;
            let flags = memory.read_i32(memory.offset(base, 24)?)?;
            (base, state, flags)
        };
        if state != 0 || flags & 2 == 0 {
            return Ok(());
        }
        let buffer = {
            let memory = host.memory();
            memory.read_pointer(memory.offset(base, 72)?)?
        };
        if let Some(buffer) = buffer {
            let synced = self.virtual_call(host, nested, buffer, 13, &[], Some(GuestStorage::Int32), vec![])?;
            if result_integer(synced)? == -1 {
                self.setstate(host.memory(), base, 4)?;
            }
        }
        Ok(())
    }
}

fn install_insertions(host: &mut WindowsServiceRegistrar<'_>, streams: &MsvcStreams) -> Result<(), GuestError> {
    {
        let streams = streams.clone();
        host.service(LIBRARY, &format!("?flush@{OSTREAM}QEAAAEAV12@XZ"), &[GuestStorage::Pointer], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, context, args| {
                let object = required_pointer(args, 0)?;
                streams.flush(ctx, context, object)?;
                Ok(ptr_value(Some(object)))
            },
        ))?;
    }
    {
        let streams = streams.clone();
        host.service(LIBRARY, &format!("?_Osfx@{OSTREAM}QEAAXXZ"), &[GuestStorage::Pointer], None, Rc::new(
            move |ctx, context, args| {
                streams.suffix(ctx, context, required_pointer(args, 0)?)?;
                Ok(GuestCallResult::Void)
            },
        ))?;
    }
    // C++ throw/unwind remains an explicit runtime stop; no exception can be
    // active while this runtime is executing normal guest instructions.
    host.service(LIBRARY, "?uncaught_exception@std@@YA_NXZ", &[], Some(GuestStorage::Uint32), Rc::new(
        move |_, _, _| Ok(GuestCallResult::Value(GuestCallValue::Uint32(0))),
    ))?;
    {
        let streams = streams.clone();
        host.service(LIBRARY, &format!("?tellp@{OSTREAM}QEAA?AV?$fpos@U_Mbstatet@@@2@XZ"), &[GuestStorage::Pointer, GuestStorage::Pointer], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, context, args| {
                let (base, result) = {
                    let memory = ctx.memory();
                    let base = streams.virtual_ios(memory, required_pointer(args, 0)?)?;
                    (base, required_pointer(args, 1)?)
                };
                let failed = {
                    let memory = ctx.memory();
                    memory.read_i32(memory.offset(base, 16)?)? & 6 != 0
                };
                if failed {
                    let memory = ctx.memory();
                    memory.write(result, &[0u8; 24])?;
                    memory.write_i64(memory.offset(result, 8)?, -1)?;
                    return Ok(ptr_value(Some(result)));
                }
                let buffer = {
                    let memory = ctx.memory();
                    ref_pointer(memory, memory.offset(base, 72)?)?
                };
                streams.virtual_call(
                    ctx,
                    context,
                    buffer,
                    10,
                    &[GuestStorage::Pointer, GuestStorage::Int64, GuestStorage::Int32, GuestStorage::Int32],
                    Some(GuestStorage::Pointer),
                    vec![
                        GuestCallValue::Pointer(Some(result)),
                        GuestCallValue::Int64(0),
                        GuestCallValue::Int32(1),
                        GuestCallValue::Int32(2),
                    ],
                )
            },
        ))?;
    }
    for bits in [32, 64] {
        let streams = streams.clone();
        let suffix = if bits == 32 { "H" } else { "_J" };
        let storage = if bits == 32 { GuestStorage::Int32 } else { GuestStorage::Int64 };
        host.service(
            LIBRARY,
            &format!("??6{OSTREAM}QEAAAEAV01@{suffix}@Z"),
            &[GuestStorage::Pointer, storage],
            Some(GuestStorage::Pointer),
            Rc::new(move |ctx, context, args| {
                let object = required_pointer(args, 0)?;
                let value = integer(args, 1)?;
                let (base, buffer) = {
                    let memory = ctx.memory();
                    let base = streams.virtual_ios(memory, object)?;
                    let buffer = memory.read_pointer(memory.offset(base, 72)?)?;
                    (base, buffer)
                };
                if let Some(buffer) = buffer {
                    streams.virtual_call(ctx, context, buffer, 1, &[], None, vec![])?;
                }
                let outcome = insert_integer(&streams, ctx, context, object, base, buffer, bits, value);
                if let Some(buffer) = buffer {
                    let unlock = streams.virtual_call(ctx, context, buffer, 2, &[], None, vec![]);
                    outcome.and(unlock.map(|_| ()))?;
                } else {
                    outcome?;
                }
                Ok(ptr_value(Some(object)))
            }),
        )?;
    }
    {
        let streams = streams.clone();
        host.service(LIBRARY, &format!("??6{OSTREAM}QEAAAEAV01@PEAV?$basic_streambuf@DU?$char_traits@D@std@@@1@@Z"), &[GuestStorage::Pointer, GuestStorage::Pointer], Some(GuestStorage::Pointer), Rc::new(
            move |ctx, context, args| {
                let object = required_pointer(args, 0)?;
                let source = pointer(args, 1)?;
                let (base, target) = {
                    let memory = ctx.memory();
                    let base = streams.virtual_ios(memory, object)?;
                    let target = memory.read_pointer(memory.offset(base, 72)?)?;
                    (base, target)
                };
                if let Some(target) = target {
                    streams.virtual_call(ctx, context, target, 1, &[], None, vec![])?;
                }
                let outcome = insert_buffer(&streams, ctx, context, object, base, target, source);
                if let Some(target) = target {
                    let unlock = streams.virtual_call(ctx, context, target, 2, &[], None, vec![]);
                    outcome.and(unlock.map(|_| ()))?;
                } else {
                    outcome?;
                }
                Ok(ptr_value(Some(object)))
            },
        ))?;
    }
    Ok(())
}

fn read_state(host: &HostCallContext, base: GuestAddress) -> Result<i32, GuestError> {
    let memory = host.memory();
    Ok(memory.read_i32(memory.offset(base, 16)?)?)
}

fn insert_integer(
    streams: &MsvcStreams,
    host: &HostCallContext,
    nested: &WindowsContext,
    object: GuestAddress,
    base: GuestAddress,
    buffer: Option<GuestAddress>,
    bits: u32,
    raw: i128,
) -> Result<(), GuestError> {
    if read_state(host, base)? == 0 {
        if let Some(buffer) = buffer {
            let tied = {
                let memory = host.memory();
                memory.read_pointer(memory.offset(base, 80)?)?
            };
            if let Some(tied) = tied {
                if tied != object {
                    streams.flush(host, nested, tied)?;
                }
            }
            if read_state(host, base)? == 0 {
                let (implementation, flags, width, fill) = {
                    let memory = host.memory();
                    let implementation = ref_pointer(memory, ref_pointer(memory, memory.offset(base, 64)?)?)?;
                    let flags = memory.read_i32(memory.offset(base, 24)?)?;
                    let width = memory.read_i64(memory.offset(base, 40)?)?;
                    let fill = memory.read_u8(memory.offset(base, 88)?)?;
                    (implementation, flags, width, fill)
                };
                let global_count = {
                    let memory = host.memory();
                    memory.read_u64(memory.offset(implementation, 24)?)?
                };
                if implementation != streams.locale.global || global_count != 0 {
                    return Err(unsupported_windows(
                        LIBRARY,
                        "ostream integer insertion",
                        "custom num_put locale requires facet dispatch",
                    ));
                }
                let basefield = flags & 0xe00;
                let radix = if basefield == 0x400 {
                    8
                } else if basefield == 0x800 {
                    16
                } else {
                    10
                };
                let value: i128 = if radix == 10 {
                    if bits == 32 { i128::from(raw as i32) } else { i128::from(raw as i64) }
                } else if bits == 32 {
                    i128::from(raw as u32)
                } else {
                    i128::from(raw as u64)
                };
                let mut digits = match radix {
                    8 => format!("{:o}", value.unsigned_abs()),
                    16 => format!("{:x}", value.unsigned_abs()),
                    _ => format!("{}", value.unsigned_abs()),
                };
                let mut prefix = if value < 0 {
                    "-".to_string()
                } else if radix == 10 && flags & 0x20 != 0 {
                    "+".to_string()
                } else {
                    String::new()
                };
                if radix != 10 && value != 0 && flags & 8 != 0 {
                    prefix = if radix == 16 { "0x".to_string() } else { "0".to_string() };
                }
                if flags & 4 != 0 {
                    digits = digits.to_uppercase();
                    prefix = prefix.to_uppercase();
                }
                if width > 0x100_0000 {
                    return Err(GuestError::invalid("MSVC numeric field exceeds limit"));
                }
                let fill = char::from(fill)
                    .to_string()
                    .repeat(0.max(width - prefix.len() as i64 - digits.len() as i64) as usize);
                let text = match flags & 0x1c0 {
                    0x40 => format!("{prefix}{digits}{fill}"),
                    0x100 => format!("{prefix}{fill}{digits}"),
                    _ => format!("{fill}{prefix}{digits}"),
                };
                for byte in text.bytes() {
                    if streams.stream_put(host, nested, buffer, i128::from(byte))? == -1 {
                        streams.setstate(host.memory(), base, 4)?;
                        break;
                    }
                }
                let memory = host.memory();
                memory.write_i64(memory.offset(base, 40)?, 0)?;
            }
        }
    }
    streams.setstate(host.memory(), base, 0)?;
    streams.suffix(host, nested, object)
}

fn insert_buffer(
    streams: &MsvcStreams,
    host: &HostCallContext,
    nested: &WindowsContext,
    object: GuestAddress,
    base: GuestAddress,
    target: Option<GuestAddress>,
    source: Option<GuestAddress>,
) -> Result<(), GuestError> {
    let mut copied = false;
    let mut state = 0;
    if read_state(host, base)? == 0 {
        if let (Some(target), Some(source)) = (target, source) {
            let tied = {
                let memory = host.memory();
                memory.read_pointer(memory.offset(base, 80)?)?
            };
            if let Some(tied) = tied {
                if tied != object {
                    streams.flush(host, nested, tied)?;
                }
            }
            if read_state(host, base)? == 0 {
                loop {
                    let character = streams.stream_get(host, nested, source, false)?;
                    if character == -1 {
                        break;
                    }
                    if streams.stream_put(host, nested, target, character)? == -1 {
                        state |= 4;
                        break;
                    }
                    streams.stream_get(host, nested, source, true)?;
                    copied = true;
                }
            }
        }
    }
    let memory = host.memory();
    memory.write_i64(memory.offset(base, 40)?, 0)?;
    streams.setstate(
        host.memory(),
        base,
        if source.is_none() { 4 } else { state | if copied { 0 } else { 2 } },
    )?;
    streams.suffix(host, nested, object)
}
