//! System V classic locale: ctype tables, facets, and caches.
//!
//! Donor: `src/guest/runtime/system-v/locale.ts` (libstdc++ 4.8
//! `locale_init.cc`, `locale_facets*.h`, GNU locale members).

use std::rc::Rc;

use crate::core::contracts::{GuestAddress, GuestCallResult, GuestCallValue, GuestStorage};
use crate::error::GuestError;
use crate::runtime::common::memory::{integer, required_pointer, write_unsigned};
use crate::core::memory::SparseGuestMemory;
use crate::runtime::system_v::contracts::{SystemVContext, SystemVServiceRegistrar};
use crate::runtime::system_v::cxx_data::{
    abi_bytes, abi_data, abi_function, abi_slot, abi_type, abi_unsupported, abi_vtable,
    CxxBase, SharedAbi,
};

fn mask(value: i32) -> u16 {
    let upper = (65..=90).contains(&value);
    let lower = (97..=122).contains(&value);
    let digit = (48..=57).contains(&value);
    let print = (32..127).contains(&value);
    let graph = value > 32 && value < 127;
    let blank = value == 32 || value == 9;
    let space = value == 32 || (9..=13).contains(&value);
    let mut flags: u16 = 0;
    if upper {
        flags |= 0x100;
    }
    if lower {
        flags |= 0x200;
    }
    if upper || lower {
        flags |= 0x400;
    }
    if digit {
        flags |= 0x800;
    }
    if digit || (65..=70).contains(&value) || (97..=102).contains(&value) {
        flags |= 0x1000;
    }
    if space {
        flags |= 0x2000;
    }
    if print {
        flags |= 0x4000;
    }
    if graph {
        flags |= 0x8000;
    }
    if blank {
        flags |= 1;
    }
    if (0..32).contains(&value) || value == 127 {
        flags |= 2;
    }
    if graph && !upper && !lower && !digit {
        flags |= 4;
    }
    if upper || lower || digit {
        flags |= 8;
    }
    flags
}

/// Constructed classic locale with its cached facets.
#[derive(Debug, Clone, Copy)]
pub struct SystemVClassicLocale {
    /// Locale implementation object.
    pub implementation: GuestAddress,
    /// C locale object.
    pub c_locale: GuestAddress,
    /// Narrow and wide ctype facets.
    pub ctype: [GuestAddress; 2],
    /// Narrow and wide num_put facets.
    pub num_put: [GuestAddress; 2],
    /// Narrow and wide num_get facets.
    pub num_get: [GuestAddress; 2],
    /// Facet base typeinfo.
    pub facet_base: GuestAddress,
}

impl SystemVClassicLocale {
    /// Retain the implementation; returns it.
    pub fn retain(
        &self,
        memory: &mut crate::core::memory::SparseGuestMemory,
    ) -> Result<GuestAddress, GuestError> {
        let count = memory.read_i32(self.implementation)?;
        memory.write_i32(self.implementation, count + 1)?;
        Ok(self.implementation)
    }

    /// Release an implementation reference.
    pub fn release(
        &self,
        memory: &mut crate::core::memory::SparseGuestMemory,
        implementation: GuestAddress,
    ) -> Result<(), GuestError> {
        let value = memory.read_i32(implementation)?;
        if value <= 2 && implementation.offset == self.implementation.offset {
            return Err(GuestError::callback("Released permanent classic locale reference"));
        }
        memory.write_i32(implementation, value - 1)?;
        Ok(())
    }
}

fn locale_facet(
    ctx: &SystemVContext,
    memory: &mut SparseGuestMemory,
    abi: &SharedAbi,
    facet_base: GuestAddress,
    name: &str,
    size: usize,
    methods: &[GuestAddress],
) -> Result<GuestAddress, GuestError> {
    let pointer = ctx.pointer_bytes as i64;
    let address = ctx.allocate(memory, size)?;
    let info = abi_type(
        ctx,
        memory,
        abi,
        name,
        &[CxxBase {
            address: facet_base,
            offset: 0,
            flags: 2,
        }],
    )?;
    let destructor = abi_unsupported(ctx, memory, &format!("__guest_{name}_destructor"))?;
    let deleting = abi_unsupported(ctx, memory, &format!("__guest_{name}_deleting_destructor"))?;
    let mut entries = vec![destructor, deleting];
    entries.extend_from_slice(methods);
    let table = abi_vtable(ctx, memory, name, info, &entries)?;
    memory.write_pointer(address, Some(table))?;
    memory.write_i32(abi_slot(memory, address, pointer)?, 1)?;
    Ok(address)
}

fn locale_text(
    ctx: &SystemVContext,
    memory: &mut SparseGuestMemory,
    value: &str,
    wide: bool,
) -> Result<GuestAddress, GuestError> {
    if !wide {
        return abi_bytes(ctx, memory, value);
    }
    let chars: Vec<char> = value.chars().collect();
    let address = ctx.allocate(memory, (chars.len() + 1) * 4)?;
    for (index, character) in chars.iter().enumerate() {
        memory
            .write_u32(abi_slot(memory, address, index as i64 * 4)?, *character as u32)?;
    }
    Ok(address)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CacheKind {
    Number,
    Money,
    Time,
}

fn locale_cache(
    ctx: &SystemVContext,
    memory: &mut SparseGuestMemory,
    abi: &SharedAbi,
    facet_base: GuestAddress,
    name: &str,
    wide: bool,
    kind: CacheKind,
) -> Result<GuestAddress, GuestError> {
    let pointer = ctx.pointer_bytes as i64;
    let width: i64 = if wide { 4 } else { 1 };
    let result = locale_facet(ctx, memory, abi, facet_base, name, 1024, &[])?;
    memory.write_i32(abi_slot(memory, result, pointer)?, 2)?;
    let mut cursor = 2 * pointer;
    macro_rules! align {
        ($n:expr) => {
            cursor = (cursor + $n - 1) / $n * $n;
        };
    }
    if kind == CacheKind::Time {
        let days = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
        let months = [
            "January", "February", "March", "April", "May", "June", "July", "August", "September",
            "October", "November", "December",
        ];
        let mut texts = vec![
            "%m/%d/%y".to_string(),
            "%m/%d/%y".to_string(),
            "%H:%M:%S".to_string(),
            "%H:%M:%S".to_string(),
            String::new(),
            String::new(),
            "AM".to_string(),
            "PM".to_string(),
            String::new(),
        ];
        for day in days {
            texts.push(day.to_string());
        }
        for day in days {
            texts.push(day[..3].to_string());
        }
        for month in months {
            texts.push(month.to_string());
        }
        for month in months {
            texts.push(month[..3].to_string());
        }
        for text in &texts {
            let address = locale_text(ctx, memory, text, wide)?;
            align!(pointer);
            memory.write_pointer(abi_slot(memory, result, cursor)?, Some(address))?;
            cursor += pointer;
        }
        return Ok(result);
    }
    // Number and money caches share the leading layout.
    let empty = abi_bytes(ctx, memory, "")?;
    align!(pointer);
    memory.write_pointer(abi_slot(memory, result, cursor)?, Some(empty))?;
    cursor += pointer;
    align!(pointer);
    write_unsigned(memory, abi_slot(memory, result, cursor)?, ctx.pointer_bytes, 0)?;
    cursor += pointer;
    cursor += 1;
    if kind == CacheKind::Number {
        for (text, size) in [("true", 4i64), ("false", 5i64)] {
            let address = locale_text(ctx, memory, text, wide)?;
            align!(pointer);
            memory.write_pointer(abi_slot(memory, result, cursor)?, Some(address))?;
            cursor += pointer;
            align!(pointer);
            write_unsigned(memory, abi_slot(memory, result, cursor)?, ctx.pointer_bytes, size as i128)?;
            cursor += pointer;
        }
        for character in [46u32, 44u32] {
            align!(width);
            write_unsigned(memory, abi_slot(memory, result, cursor)?, width as usize, i128::from(character))?;
            cursor += width;
        }
        for text in ["-+xX0123456789abcdef0123456789ABCDEF", "-+xX0123456789abcdefABCDEF"] {
            for character in text.chars() {
                align!(width);
                write_unsigned(memory, abi_slot(memory, result, cursor)?, width as usize, i128::from(character as u32))?;
                cursor += width;
            }
        }
    } else {
        for character in [46u32, 44u32] {
            align!(width);
            write_unsigned(memory, abi_slot(memory, result, cursor)?, width as usize, i128::from(character))?;
            cursor += width;
        }
        for _ in 0..3 {
            let address = locale_text(ctx, memory, "", wide)?;
            align!(pointer);
            memory.write_pointer(abi_slot(memory, result, cursor)?, Some(address))?;
            cursor += pointer;
            align!(pointer);
            write_unsigned(memory, abi_slot(memory, result, cursor)?, ctx.pointer_bytes, 0)?;
            cursor += pointer;
        }
        align!(4);
        cursor += 4;
        memory.write(abi_slot(memory, result, cursor)?, &[2, 3, 0, 4, 2, 3, 0, 4])?;
        cursor += 8;
        for character in "-0123456789".chars() {
            align!(width);
            write_unsigned(memory, abi_slot(memory, result, cursor)?, width as usize, i128::from(character as u32))?;
            cursor += width;
        }
    }
    Ok(result)
}

fn ceil_div(value: i64, n: i64) -> i64 {
    (value + n - 1) / n
}

fn ctype_methods(
    ctx: &SystemVContext,
    memory: &mut SparseGuestMemory,
    wide: bool,
) -> Result<Vec<GuestAddress>, GuestError> {
    let pointer_bytes = ctx.pointer_bytes;
    let pointer = pointer_bytes as i64;
    let character_storage = if wide { GuestStorage::Uint32 } else { GuestStorage::Int32 };
    let ch = if wide { "w" } else { "c" };
    let width: i64 = if wide { 4 } else { 1 };
    let mut methods = Vec::new();
    if wide {
        let method = abi_function(
            ctx,
            memory,
            &format!("__guest_ctype_{ch}_is"),
            &[GuestStorage::Pointer, GuestStorage::Uint32, GuestStorage::Uint32],
            Some(GuestStorage::Int32),
            Rc::new(move |ctx, _, args| {
                let memory = ctx.memory();
                let object = required_pointer(args, 0)?;
                let requested = integer(args, 1)? as u32;
                let value = integer(args, 2)? as u32;
                let widen = ceil_div(3 * pointer + 129, 4) * 4;
                let masks = ceil_div(widen + 1056, pointer) * pointer;
                for bit in 0..12 {
                    let flag = memory.read_u16(memory.offset(object, widen + 1024 + bit * 2)?)? as u32;
                    if flag & requested == 0 {
                        continue;
                    }
                    let Some(table) = memory.read_pointer(memory.offset(object, masks + bit * pointer)?)? else {
                        continue;
                    };
                    let index1 = value >> memory.read_u32(table)?;
                    if index1 >= memory.read_u32(memory.offset(table, 4)?)? {
                        continue;
                    }
                    let lookup1 = memory.read_u32(memory.offset(table, 20 + index1 as i64 * 4)?)?;
                    if lookup1 == 0 {
                        continue;
                    }
                    let index2 = (value >> memory.read_u32(memory.offset(table, 8)?)?)
                        & memory.read_u32(memory.offset(table, 12)?)?;
                    let lookup2 = memory.read_u32(memory.offset(table, lookup1 as i64 + index2 as i64 * 4)?)?;
                    if lookup2 == 0 {
                        continue;
                    }
                    let index3 = (value >> 5) & memory.read_u32(memory.offset(table, 16)?)?;
                    let word = memory.read_u32(memory.offset(table, lookup2 as i64 + index3 as i64 * 4)?)?;
                    if ((word >> (value & 31)) & 1) != 0 {
                        return Ok(GuestCallResult::Value(GuestCallValue::Int32(1)));
                    }
                }
                Ok(GuestCallResult::Value(GuestCallValue::Int32(0)))
            }),
            "GLIBCXX_3.4",
        )?;
        methods.push(method);
        for operation in ["is_range", "scan_is", "scan_not"] {
            methods.push(abi_unsupported(ctx, memory, &format!("__guest_ctype_{ch}_{operation}"))?);
        }
    }
    for operation in ["toupper", "tolower"] {
        let to_upper = operation == "toupper";
        let convert_name = format!("__guest_ctype_{ch}_{operation}");
        let method = abi_function(
            ctx,
            memory,
            &convert_name,
            &[GuestStorage::Pointer, character_storage],
            Some(character_storage),
            Rc::new(move |ctx, _, args| {
                let memory = ctx.memory();
                let object = required_pointer(args, 0)?;
                let c = integer(args, 1)?;
                let value = if wide {
                    let c = c as u32;
                    if to_upper {
                        if (97..=122).contains(&c) { c - 32 } else { c }
                    } else if (65..=90).contains(&c) {
                        c + 32
                    } else {
                        c
                    }
                } else {
                    let table = memory
                        .read_pointer(memory.offset(object, (if to_upper { 4 } else { 5 }) * pointer)?)?
                        .ok_or_else(|| GuestError::invalid("Missing guest ctype conversion table"))?;
                    let raw = memory.read_i32(memory.offset(table, ((c & 255) * 4) as i64)?)?;
                    ((raw << 24) >> 24) as u32
                };
                Ok(if wide {
                    GuestCallResult::Value(GuestCallValue::Uint32(value))
                } else {
                    GuestCallResult::Value(GuestCallValue::Int32(value as i32))
                })
            }),
            "GLIBCXX_3.4",
        )?;
        methods.push(method);
        let range_name = format!("__guest_ctype_{ch}_{operation}_range");
        let method = abi_function(
            ctx,
            memory,
            &range_name,
            &[GuestStorage::Pointer, GuestStorage::Pointer, GuestStorage::Pointer],
            Some(GuestStorage::Pointer),
            Rc::new(move |ctx, _, args| {
                let memory = ctx.memory();
                let object = required_pointer(args, 0)?;
                let mut current = required_pointer(args, 1)?;
                let end = required_pointer(args, 2)?;
                while current.offset < end.offset {
                    let c = if wide {
                        memory.read_u32(current)? as i128
                    } else {
                        i128::from(memory.read_u8(current)?)
                    };
                    let value = if wide {
                        let c = c as u32;
                        if to_upper {
                            if (97..=122).contains(&c) { c - 32 } else { c }
                        } else if (65..=90).contains(&c) {
                            c + 32
                        } else {
                            c
                        }
                    } else {
                        let table = memory
                            .read_pointer(memory.offset(object, (if to_upper { 4 } else { 5 }) * pointer)?)?
                            .ok_or_else(|| GuestError::invalid("Missing guest ctype conversion table"))?;
                        let raw = memory.read_i32(memory.offset(table, ((c & 255) * 4) as i64)?)?;
                        ((raw << 24) >> 24) as u32
                    };
                    if wide {
                        memory.write_u32(current, value)?;
                    } else {
                        memory.write_u8(current, (value & 0xff) as u8)?;
                    }
                    current = memory.offset(current, width)?;
                }
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(end))))
            }),
            "GLIBCXX_3.4",
        )?;
        methods.push(method);
    }
    {
        let method = abi_function(
            ctx,
            memory,
            &format!("__guest_ctype_{ch}_widen"),
            &[GuestStorage::Pointer, GuestStorage::Int32],
            Some(character_storage),
            Rc::new(move |ctx, _, args| {
                let memory = ctx.memory();
                let object = required_pointer(args, 0)?;
                let input = (integer(args, 1)? & 255) as u32;
                let value = if wide {
                    let widen = ceil_div(3 * pointer + 129, 4) * 4;
                    memory.read_u32(memory.offset(object, widen + input as i64 * 4)?)?
                } else {
                    (((input << 24) as i32) >> 24) as u32
                };
                Ok(if wide {
                    GuestCallResult::Value(GuestCallValue::Uint32(value))
                } else {
                    GuestCallResult::Value(GuestCallValue::Int32(value as i32))
                })
            }),
            "GLIBCXX_3.4",
        )?;
        methods.push(method);
    }
    {
        let method = abi_function(
            ctx,
            memory,
            &format!("__guest_ctype_{ch}_widen_range"),
            &[GuestStorage::Pointer, GuestStorage::Pointer, GuestStorage::Pointer, GuestStorage::Pointer],
            Some(GuestStorage::Pointer),
            Rc::new(move |ctx, _, args| {
                let memory = ctx.memory();
                let object = required_pointer(args, 0)?;
                let mut source = required_pointer(args, 1)?;
                let mut target = required_pointer(args, 3)?;
                let end = required_pointer(args, 2)?;
                while source.offset < end.offset {
                    let input = u32::from(memory.read_u8(source)?);
                    let value = if wide {
                        let widen = ceil_div(3 * pointer + 129, 4) * 4;
                        memory.read_u32(memory.offset(object, widen + input as i64 * 4)?)?
                    } else {
                        (((input << 24) as i32) >> 24) as u32
                    };
                    if wide {
                        memory.write_u32(target, value)?;
                    } else {
                        memory.write_u8(target, (value & 0xff) as u8)?;
                    }
                    source = memory.offset(source, 1)?;
                    target = memory.offset(target, width)?;
                }
                Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(end))))
            }),
            "GLIBCXX_3.4",
        )?;
        methods.push(method);
    }
    {
        let method = abi_function(
            ctx,
            memory,
            &format!("__guest_ctype_{ch}_narrow"),
            &[GuestStorage::Pointer, character_storage, GuestStorage::Int32],
            Some(GuestStorage::Int32),
            Rc::new(move |ctx, _, args| {
                let memory = ctx.memory();
                let object = required_pointer(args, 0)?;
                let input = integer(args, 1)? as u32;
                let narrow = 3 * pointer;
                let value = if wide
                    && input < 128
                    && memory.read_u8(memory.offset(object, narrow)?)? != 0
                {
                    u32::from(memory.read_u8(memory.offset(object, narrow + 1 + input as i64)?)?)
                } else if wide && input > 127 {
                    integer(args, 2)? as u32
                } else {
                    input
                };
                Ok(GuestCallResult::Value(GuestCallValue::Int32(
                    (value as u8) as i8 as i32,
                )))
            }),
            "GLIBCXX_3.4",
        )?;
        methods.push(method);
    }
    methods.push(abi_unsupported(ctx, memory, &format!("__guest_ctype_{ch}_narrow_range"))?);
    Ok(methods)
}

struct FacetSet {
    ctype: GuestAddress,
    num_get: GuestAddress,
    num_put: GuestAddress,
}

fn construct_facets(
    ctx: &SystemVContext,
    memory: &mut SparseGuestMemory,
    abi: &SharedAbi,
    locale: &SystemVClassicLocale,
    wide: bool,
    facets: GuestAddress,
    caches: GuestAddress,
    classification: GuestAddress,
    lower: GuestAddress,
    upper: GuestAddress,
) -> Result<FacetSet, GuestError> {
    let pointer = ctx.pointer_bytes as i64;
    let ch = if wide { "w" } else { "c" };
    let names = [
        format!("St5ctypeI{ch}E"),
        format!("St7codecvtI{ch}c11__mbstate_tE"),
        format!("St8numpunctI{ch}E"),
        format!("St7num_getI{ch}St19istreambuf_iteratorI{ch}St11char_traitsI{ch}EEE"),
        format!("St7num_putI{ch}St19ostreambuf_iteratorI{ch}St11char_traitsI{ch}EEE"),
        format!("St7collateI{ch}E"),
        format!("St10moneypunctI{ch}Lb0EE"),
        format!("St10moneypunctI{ch}Lb1EE"),
        format!("St9money_getI{ch}St19istreambuf_iteratorI{ch}St11char_traitsI{ch}EEE"),
        format!("St9money_putI{ch}St19ostreambuf_iteratorI{ch}St11char_traitsI{ch}EEE"),
        format!("St11__timepunctI{ch}E"),
        format!("St8time_getI{ch}St19istreambuf_iteratorI{ch}St11char_traitsI{ch}EEE"),
        format!("St8time_putI{ch}St19ostreambuf_iteratorI{ch}St11char_traitsI{ch}EEE"),
        format!("St8messagesI{ch}E"),
    ];
    let mut built = Vec::new();
    for (index, name) in names.iter().enumerate() {
        let methods = if index == 0 {
            ctype_methods(ctx, memory, wide)?
        } else {
            let mut unsupported = Vec::new();
            for slot in 0..14 {
                unsupported.push(abi_unsupported(ctx, memory, &format!("__guest_{name}_virtual_{slot}"))?);
            }
            unsupported
        };
        let size = if index == 0 {
            if wide {
                if ctx.pointer_bytes == 4 { 1264 } else { 1344 }
            } else {
                ceil_div(7 * pointer + 514, pointer) as usize * pointer as usize
            }
        } else if index == 10 {
            5 * pointer as usize
        } else if index == 13 {
            4 * pointer as usize
        } else if [1, 2, 5, 6, 7].contains(&index) {
            3 * pointer as usize
        } else {
            2 * pointer as usize
        };
        let facet = locale_facet(ctx, memory, abi, locale.facet_base, name, size, &methods)?;
        let id = (if wide { 14 } else { 0 }) + index;
        built.push(facet);
        memory.write_pointer(abi_slot(memory, facets, id as i64 * pointer)?, Some(facet))?;
        memory.write_i32(abi_slot(memory, facet, pointer)?, 2)?;
        let id_symbol = format!("_ZN{name}2idE");
        let id_address = abi_data(ctx, memory, &id_symbol, pointer as usize, "GLIBCXX_3.4")?;
        write_unsigned(memory, id_address, ctx.pointer_bytes, (id + 1) as i128)?;
        if index == 0 {
            memory.write_pointer(abi_slot(memory, facet, 2 * pointer)?, Some(locale.c_locale))?;
            if !wide {
                memory.write_pointer(abi_slot(memory, facet, 4 * pointer)?, Some(upper))?;
                memory.write_pointer(abi_slot(memory, facet, 5 * pointer)?, Some(lower))?;
                memory.write_pointer(abi_slot(memory, facet, 6 * pointer)?, Some(classification))?;
            } else {
                let narrow = 3 * pointer;
                memory.write_u8(abi_slot(memory, facet, narrow)?, 1)?;
                for c in 0..128 {
                    memory.write_u8(abi_slot(memory, facet, narrow + 1 + c)?, c as u8)?;
                }
                let widen = ceil_div(narrow + 129, 4) * 4;
                for c in 0..256 {
                    memory.write_u32(
                        abi_slot(memory, facet, widen + c * 4)?,
                        if c < 128 { c as u32 } else { 0xffff_ffff },
                    )?;
                }
                let masks = ceil_div(widen + 1056, pointer) * pointer;
                for bit in 0..12 {
                    let flag: u32 = if bit < 8 { 1 << (bit + 8) } else { 1 << (bit - 8) };
                    memory.write_u16(abi_slot(memory, facet, widen + 1024 + bit * 2)?, flag as u16)?;
                    // GCC 4.8 _M_convert_to_wmask does not map the C library's blank bit.
                    if bit == 8 {
                        continue;
                    }
                    // GLIBC's three-level wctype bit-table layout, retaining the C-locale ASCII domain.
                    let table = ctx.allocate(memory, 56)?;
                    for (slot, value) in [7u32, 1, 5, 3, 0, 24].iter().enumerate() {
                        memory.write_u32(abi_slot(memory, table, slot as i64 * 4)?, *value)?;
                    }
                    for group in 0..4 {
                        memory.write_u32(abi_slot(memory, table, 24 + group * 4)?, (40 + group * 4) as u32)?;
                        let mut bits = 0u32;
                        for offset in 0..32 {
                            if mask((group * 32 + offset) as i32) as u32 & flag != 0 {
                                bits |= 1 << offset;
                            }
                        }
                        memory.write_u32(abi_slot(memory, table, 40 + group * 4)?, bits)?;
                    }
                    memory.write_pointer(abi_slot(memory, facet, masks + bit * pointer)?, Some(table))?;
                }
            }
        } else if [2, 6, 7, 10].contains(&index) {
            let cache_name = if index == 2 {
                format!("St16__numpunct_cacheI{ch}E")
            } else if index == 10 {
                format!("St17__timepunct_cacheI{ch}E")
            } else {
                format!("St18__moneypunct_cacheI{ch}Lb{}EE", if index == 7 { 1 } else { 0 })
            };
            let kind = if index == 2 {
                CacheKind::Number
            } else if index == 10 {
                CacheKind::Time
            } else {
                CacheKind::Money
            };
            let cache = locale_cache(ctx, memory, abi, locale.facet_base, &cache_name, wide, kind)?;
            memory.write_pointer(abi_slot(memory, facet, 2 * pointer)?, Some(cache))?;
            memory.write_pointer(abi_slot(memory, caches, id as i64 * pointer)?, Some(cache))?;
            if index == 10 {
                memory.write_pointer(abi_slot(memory, facet, 3 * pointer)?, Some(locale.c_locale))?;
                let name = abi_bytes(ctx, memory, "C")?;
                memory.write_pointer(abi_slot(memory, facet, 4 * pointer)?, Some(name))?;
            }
        } else if [1, 5, 13].contains(&index) {
            memory.write_pointer(abi_slot(memory, facet, 2 * pointer)?, Some(locale.c_locale))?;
            if index == 13 {
                let name = abi_bytes(ctx, memory, "C")?;
                memory.write_pointer(abi_slot(memory, facet, 3 * pointer)?, Some(name))?;
            }
        }
    }
    let (Some(ctype), Some(num_get), Some(num_put)) =
        (built.first().copied(), built.get(3).copied(), built.get(4).copied())
    else {
        return Err(GuestError::callback("Missing standard C locale facets"));
    };
    Ok(FacetSet { ctype, num_get, num_put })
}

/// Build the classic locale and its facets.
pub fn build_classic_locale(
    host: &mut SystemVServiceRegistrar<'_>,
    abi: &SharedAbi,
) -> Result<SystemVClassicLocale, GuestError> {
    let ctx = &host.context;
    let memory = &mut *host.memory;
    let pointer = ctx.pointer_bytes as i64;
    let facet_base = abi_type(ctx, memory, abi, "NSt6locale5facetE", &[])?;
    let c_locale = ctx.allocate(memory, 29 * pointer as usize)?;
    let tables = ctx.allocate(memory, 384 * 10)?;
    let classification = abi_slot(memory, tables, 128 * 2)?;
    let lower = abi_slot(memory, tables, 384 * 2 + 128 * 4)?;
    let upper = abi_slot(memory, tables, 384 * 6 + 128 * 4)?;
    for c in -128..256 {
        let byte = if c < 0 && c != -1 { c + 256 } else { c };
        memory.write_u16(abi_slot(memory, classification, c as i64 * 2)?, mask(byte))?;
        memory.write_i32(
            abi_slot(memory, lower, c as i64 * 4)?,
            if (65..=90).contains(&byte) { byte + 32 } else { byte },
        )?;
        memory.write_i32(
            abi_slot(memory, upper, c as i64 * 4)?,
            if (97..=122).contains(&byte) { byte - 32 } else { byte },
        )?;
    }
    memory.write_pointer(abi_slot(memory, c_locale, 13 * pointer)?, Some(classification))?;
    memory.write_pointer(abi_slot(memory, c_locale, 14 * pointer)?, Some(lower))?;
    memory.write_pointer(abi_slot(memory, c_locale, 15 * pointer)?, Some(upper))?;
    let c_name = abi_bytes(ctx, memory, "C")?;
    for index in 0..13 {
        memory.write_pointer(abi_slot(memory, c_locale, (16 + index) * pointer)?, Some(c_name))?;
    }
    for (name, table) in [
        ("__ctype_b_loc", classification),
        ("__ctype_tolower_loc", lower),
        ("__ctype_toupper_loc", upper),
    ] {
        let slot = ctx.allocate(memory, pointer as usize)?;
        memory.write_pointer(slot, Some(table))?;
        ctx.service(memory, "libc.so.6", name, &[Some("GLIBC_2.3"), None], &[], Some(GuestStorage::Pointer), Rc::new(
            move |_, _, _| Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(slot)))),
        ))?;
    }
    let facets = ctx.allocate(memory, 28 * pointer as usize)?;
    let caches = ctx.allocate(memory, 28 * pointer as usize)?;
    let names = ctx.allocate(memory, 12 * pointer as usize)?;
    memory.write_pointer(names, Some(c_name))?;
    let implementation = ctx.allocate(memory, 5 * pointer as usize)?;
    memory.write_i32(implementation, 2)?;
    memory.write_pointer(abi_slot(memory, implementation, pointer)?, Some(facets))?;
    write_unsigned(memory, abi_slot(memory, implementation, 2 * pointer)?, ctx.pointer_bytes, 28)?;
    memory.write_pointer(abi_slot(memory, implementation, 3 * pointer)?, Some(caches))?;
    memory.write_pointer(abi_slot(memory, implementation, 4 * pointer)?, Some(names))?;
    let mut locale = SystemVClassicLocale {
        implementation,
        c_locale,
        ctype: [classification, classification],
        num_put: [classification, classification],
        num_get: [classification, classification],
        facet_base,
    };
    let narrow = construct_facets(ctx, memory, abi, &locale, false, facets, caches, classification, lower, upper)?;
    let wide = construct_facets(ctx, memory, abi, &locale, true, facets, caches, classification, lower, upper)?;
    locale.ctype = [narrow.ctype, wide.ctype];
    locale.num_put = [narrow.num_put, wide.num_put];
    locale.num_get = [narrow.num_get, wide.num_get];
    Ok(locale)
}
