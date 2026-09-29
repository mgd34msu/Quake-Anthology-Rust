//! ELF image loading: mapping, eager relocation, metadata collection.
//!
//! Donor: the loader half of `src/guest/elf/index.ts` (`loadElf`). Maps and
//! eagerly relocates one image; the caller loads dependencies and executes
//! initializers.

use std::collections::{HashMap, HashSet};

use crate::core::contracts::{
    GuestAddress, GuestExport, GuestExportTarget, GuestImage, GuestImport, GuestImportResolver, GuestMapping,
    GuestPermissions, GuestSymbolName, GuestTlsTemplate, ModuleIdentity,
};
use crate::core::memory::SparseGuestMemory;
use crate::elf::parse::{checked_number, dynamic_value, elf_file_offset, inspect_elf, ElfInspection, ElfSymbol};
use crate::elf::relocate::{
    defined_symbol_address, elf_address, relocate_elf, resolve_symbol, ElfRelocationContext, ElfTlsBindings,
    IndirectResolver, SymbolSizeResolver,
};
use crate::elf::unwind::read_elf_unwind;
use crate::error::GuestError;

/// Loaded ELF image with loader metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct ElfGuestImage {
    /// Base guest image.
    pub image: GuestImage,
    /// ELF inspection.
    pub elf: ElfInspection,
    /// Load bias.
    pub load_bias: u64,
    /// Needed libraries.
    pub needed_libraries: Vec<String>,
    /// SONAME, if any.
    pub soname: Option<String>,
    /// Executable-stack flag, if declared.
    pub executable_stack: Option<bool>,
    /// Pre-initializers (main executable only).
    pub preinitializers: Vec<GuestAddress>,
    /// TLS exports: STT_TLS values are block offsets, never addresses.
    pub tls_exports: Vec<ElfTlsExport>,
}

/// One TLS export: symbol, block offset, and length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfTlsExport {
    /// Exported symbol.
    pub symbol: GuestSymbolName,
    /// Block offset.
    pub offset: u64,
    /// Length in bytes.
    pub byte_length: usize,
}

/// ELF load options.
pub struct ElfLoadOptions<'a> {
    /// Image bytes.
    pub bytes: &'a [u8],
    /// Owning module.
    pub module: ModuleIdentity,
    /// Guest memory.
    pub memory: &'a mut SparseGuestMemory,
    /// Additive load bias. ET_EXEC requires zero; ET_DYN normally nonzero.
    pub load_bias: u64,
    /// Import resolver.
    pub resolver: &'a dyn GuestImportResolver,
    /// Names already provided by the image graph or runtime services.
    pub dependencies: &'a HashSet<String>,
    /// TLS bindings, if any.
    pub tls: Option<&'a dyn ElfTlsBindings>,
    /// Guest IFUNC resolver execution; must never use native execution.
    pub resolve_indirect: Option<&'a dyn IndirectResolver>,
    /// Provider symbol sizes for COPY/SIZE relocations.
    pub resolve_symbol_size: Option<&'a dyn SymbolSizeResolver>,
    /// One registry per guest process.
    pub unique_symbols: Option<&'a mut HashMap<String, GuestAddress>>,
}

const PAGE: u64 = 4096;

fn down(value: u64) -> u64 {
    value - value % PAGE
}

fn up(value: u64) -> u64 {
    value.next_multiple_of(PAGE)
}

fn permissions(flags: u32) -> GuestPermissions {
    if flags & 2 != 0 {
        if flags & 1 != 0 {
            GuestPermissions::ReadWriteExecute
        } else {
            GuestPermissions::ReadWrite
        }
    } else if flags & 1 != 0 {
        if flags & 4 != 0 {
            GuestPermissions::ReadExecute
        } else {
            GuestPermissions::Execute
        }
    } else if flags & 4 != 0 {
        GuestPermissions::Read
    } else {
        GuestPermissions::None
    }
}

fn elf_error(detail: impl Into<String>) -> GuestError {
    GuestError::bad_image("elf", detail)
}

/// Map and eagerly relocate one image.
pub fn load_elf(options: ElfLoadOptions) -> Result<ElfGuestImage, GuestError> {
    let bytes = options.bytes.to_vec();
    let elf = inspect_elf(&bytes)?;
    let ElfLoadOptions {
        module,
        memory,
        load_bias,
        resolver,
        dependencies,
        tls,
        resolve_indirect,
        resolve_symbol_size,
        mut unique_symbols,
        ..
    } = options;
    if elf.symbols.iter().any(|symbol| symbol.binding == 10) && unique_symbols.is_none() {
        return Err(elf_error("GNU unique symbols require a process-wide symbol registry"));
    }
    if elf.abi.pointer_bytes() != memory.pointer_bytes() {
        return Err(elf_error("image and guest memory have different pointer widths"));
    }
    if (elf.image_type == crate::elf::parse::ElfType::Executable && load_bias != 0) || load_bias % PAGE != 0 {
        return Err(elf_error("invalid load bias for fixed ELF executable"));
    }
    for library in &elf.needed_libraries {
        if !dependencies.contains(library) {
            return Err(elf_error(format!("dependency {library} has no guest/runtime provider")));
        }
    }
    validate_dynamic_policy(&elf)?;
    let supported: HashSet<u32> = [0, 1, 2, 3, 4, 6, 7, 0x6474_e550, 0x6474_e551, 0x6474_e552]
        .into_iter()
        .collect();
    for segment in &elf.segments {
        if !supported.contains(&segment.segment_type) {
            return Err(elf_error(format!(
                "unsupported program segment 0x{:x}",
                segment.segment_type
            )));
        }
        if (segment.segment_type == 1 || segment.segment_type == 7)
            && segment.alignment > 1
            && load_bias % segment.alignment != 0
        {
            return Err(elf_error("load bias violates segment alignment"));
        }
        if segment.segment_type == 1 && segment.address.wrapping_sub(segment.offset as u64) % PAGE != 0 {
            return Err(elf_error("PT_LOAD is not page congruent"));
        }
        if segment.segment_type == 1 && segment.flags & !7 != 0 {
            return Err(elf_error("unsupported PT_LOAD permission flags"));
        }
    }
    let load_segments: Vec<&crate::elf::parse::ElfSegment> = elf
        .segments
        .iter()
        .filter(|segment| segment.segment_type == 1 && segment.memory_size > 0)
        .collect();
    let mut boundaries: Vec<u64> = load_segments
        .iter()
        .flat_map(|segment| [down(segment.address), up(segment.address + segment.memory_size as u64)])
        .collect();
    boundaries.sort_unstable();
    boundaries.dedup();
    let (Some(first), Some(last)) = (boundaries.first(), boundaries.last()) else {
        return Err(elf_error("no image span"));
    };
    let (first, last) = (*first, *last);
    let span = last - first;
    let mut planned: Vec<GuestMapping> = Vec::new();
    for window in boundaries.windows(2) {
        let (start, end) = (window[0], window[1]);
        let overlapping: Vec<&&crate::elf::parse::ElfSegment> = load_segments
            .iter()
            .filter(|segment| down(segment.address) <= start && up(segment.address + segment.memory_size as u64) >= end)
            .collect();
        if overlapping.is_empty() {
            continue;
        }
        let flags = overlapping[overlapping.len() - 1].flags;
        planned.push(GuestMapping {
            base: load_bias + start,
            byte_length: checked_number(end - start, "mapping length")?,
            permissions: permissions(flags),
            label: format!("{}:PT_LOAD", module.artifact_path),
        });
    }
    let prior_unique: HashSet<String> = unique_symbols
        .as_ref()
        .map_or(HashSet::new(), |registry| registry.keys().cloned().collect());
    let mut mapped: Vec<GuestMapping> = Vec::new();
    let result = load_inner(
        &bytes,
        &elf,
        module.clone(),
        &mut *memory,
        load_bias,
        resolver,
        tls,
        resolve_indirect,
        resolve_symbol_size,
        unique_symbols.as_deref_mut(),
        &planned,
        &load_segments,
        &mut mapped,
        first,
        span,
    );
    match result {
        Ok(image) => Ok(image),
        Err(error) => {
            if let Some(registry) = unique_symbols.as_mut() {
                let added: Vec<String> = registry
                    .keys()
                    .filter(|name| !prior_unique.contains(*name))
                    .cloned()
                    .collect();
                for name in added {
                    registry.remove(&name);
                }
            }
            for mapping in &mapped {
                let address = elf_address(memory, mapping.base)?;
                memory.unmap(address, mapping.byte_length)?;
            }
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn load_inner(
    bytes: &[u8],
    elf: &ElfInspection,
    module: ModuleIdentity,
    memory: &mut SparseGuestMemory,
    load_bias: u64,
    resolver: &dyn GuestImportResolver,
    tls_bindings: Option<&dyn ElfTlsBindings>,
    resolve_indirect: Option<&dyn IndirectResolver>,
    resolve_symbol_size: Option<&dyn SymbolSizeResolver>,
    unique_symbols: Option<&mut HashMap<String, GuestAddress>>,
    planned: &[GuestMapping],
    load_segments: &[&crate::elf::parse::ElfSegment],
    mapped: &mut Vec<GuestMapping>,
    first: u64,
    span: u64,
) -> Result<ElfGuestImage, GuestError> {
    for mapping in planned {
        memory.map(&crate::core::contracts::GuestMapOptions {
            base: mapping.base,
            byte_length: mapping.byte_length,
            permissions: GuestPermissions::ReadWriteExecute,
            label: mapping.label.clone(),
            bytes: None,
        })?;
        mapped.push(mapping.clone());
    }
    for segment in load_segments {
        // Linux maps the initial file page, including bytes preceding an
        // unaligned p_vaddr.
        let prefix = checked_number(segment.address - down(segment.address), "segment page prefix")?;
        if segment.offset < prefix {
            return Err(elf_error("segment page precedes the input file"));
        }
        let file_start = segment.offset - prefix;
        if segment.file_size > 0 {
            let initialized = bytes
                .get(file_start..segment.offset + segment.file_size)
                .ok_or_else(|| elf_error("segment file range exceeds input"))?;
            memory.write(elf_address(memory, load_bias + down(segment.address))?, initialized)?;
        }
        if segment.memory_size > segment.file_size {
            memory.write(
                elf_address(memory, load_bias + segment.address + segment.file_size as u64)?,
                &vec![0u8; segment.memory_size - segment.file_size],
            )?;
        }
    }
    let mut imports: Vec<GuestImport> = Vec::new();
    let mut exports: Vec<GuestExport> = Vec::new();
    let tls_segment = elf.segments.iter().find(|segment| segment.segment_type == 7);
    for symbol in &elf.symbols {
        if symbol.symbol_type == 6 && symbol.section != 0 {
            match tls_segment {
                Some(tls_segment) if symbol.value + symbol.size as u64 <= tls_segment.memory_size as u64 => {}
                _ => {
                    return Err(elf_error(format!("TLS symbol {} exceeds its template", symbol.name)));
                }
            }
        }
    }
    let stack = elf.segments.iter().find(|segment| segment.segment_type == 0x6474_e551);
    let symbol_name = |symbol: &ElfSymbol| GuestSymbolName::Name {
        name: symbol.name.clone(),
        version: symbol.version.as_ref().map(|version| version.name.clone()),
    };
    let exported: Vec<&ElfSymbol> = elf
        .symbols
        .iter()
        .filter(|symbol| {
            !symbol.name.is_empty()
                && symbol.section != 0
                && [1, 2, 10].contains(&symbol.binding)
                && symbol.visibility != 1
                && symbol.visibility != 2
        })
        .collect();
    let mut append_export = |symbol: &ElfSymbol, address: GuestAddress| {
        exports.push(GuestExport {
            symbol: symbol_name(symbol),
            target: GuestExportTarget::Address(address),
        });
        if symbol.version.as_ref().is_some_and(|version| !version.hidden) {
            exports.push(GuestExport {
                symbol: GuestSymbolName::Name {
                    name: symbol.name.clone(),
                    version: None,
                },
                target: GuestExportTarget::Address(address),
            });
        }
    };
    for symbol in &exported {
        if symbol.symbol_type == 6 || symbol.symbol_type == 10 || symbol.binding == 10 {
            continue;
        }
        let address = defined_symbol_address(memory, load_bias, resolve_indirect, symbol)?;
        append_export(symbol, elf_address(memory, address)?);
    }
    let mut image = GuestImage {
        module: module.clone(),
        abi: elf.abi,
        base: elf_address(memory, load_bias + first)?,
        preferred_base: first,
        byte_length: span,
        entry_point: if elf.entry_point == 0 {
            None
        } else {
            Some(elf_address(memory, load_bias + elf.entry_point)?)
        },
        mappings: planned.to_vec(),
        imports: Vec::new(),
        exports: Vec::new(),
        tls: tls_segment.map(|tls_segment| GuestTlsTemplate {
            image: module.clone(),
            initialized: bytes
                .get(tls_segment.offset..tls_segment.offset + tls_segment.file_size)
                .unwrap_or(&[])
                .to_vec(),
            zero_fill_bytes: tls_segment.memory_size - tls_segment.file_size,
            alignment: if tls_segment.alignment > 0 {
                tls_segment.alignment
            } else {
                1
            },
            callbacks: Vec::new(),
        }),
        initializers: Vec::new(),
        finalizers: Vec::new(),
        unwind: Vec::new(),
    };
    let mut relocation_context = ElfRelocationContext {
        elf,
        memory,
        image: &image,
        load_bias,
        resolver,
        tls: tls_bindings,
        resolve_indirect,
        resolve_symbol_size,
        unique_symbols,
        imports: &mut imports,
    };
    // Borrow dance: unique-symbol exports resolve before relocation.
    // (Split into a scoped block to satisfy the borrow checker.)
    {
        let context = &mut relocation_context;
        for symbol in &exported {
            if symbol.binding != 10 {
                continue;
            }
            if symbol.symbol_type != 0 && symbol.symbol_type != 1 {
                return Err(elf_error("GNU unique binding is only supported for data symbols"));
            }
            let base = context.image.base;
            let address = resolve_symbol(context, symbol, base, false)?;
            let memory: &mut SparseGuestMemory = &mut *context.memory;
            append_export(symbol, elf_address(memory, address)?);
        }
    }
    relocate_elf(&mut relocation_context)?;
    // IFUNC exports resolve after relocation.
    let ElfRelocationContext {
        memory,
        tls: _,
        resolve_indirect,
        ..
    } = relocation_context;
    for symbol in &exported {
        if symbol.symbol_type != 10 {
            continue;
        }
        let address = defined_symbol_address(memory, load_bias, resolve_indirect, symbol)?;
        append_export(symbol, elf_address(memory, address)?);
    }
    let preinitializers = pointer_array(elf, memory, load_bias, 32, 33)?;
    let main_executable = elf.image_type == crate::elf::parse::ElfType::Executable
        || elf.interpreter.is_some()
        || dynamic_value(&elf.dynamic, 0x6fff_fffb).unwrap_or(0) & 0x0800_0000 != 0;
    if !main_executable && !preinitializers.is_empty() {
        return Err(elf_error("DT_PREINIT_ARRAY is only valid for the main executable"));
    }
    let init = dynamic_value(&elf.dynamic, 12);
    let fini = dynamic_value(&elf.dynamic, 13);
    let mut initializers = Vec::new();
    if !matches!(init, None | Some(0)) {
        initializers.push(elf_address(memory, load_bias + init.unwrap_or(0))?);
    }
    initializers.extend(pointer_array(elf, memory, load_bias, 25, 27)?);
    let mut finalizers = pointer_array(elf, memory, load_bias, 26, 28)?;
    finalizers.reverse();
    if !matches!(fini, None | Some(0)) {
        finalizers.push(elf_address(memory, load_bias + fini.unwrap_or(0))?);
    }
    let unwind = read_elf_unwind(elf, bytes, memory, load_bias)?;
    let tls = match (&image.tls, tls_segment) {
        (Some(template), Some(tls_segment)) => Some(GuestTlsTemplate {
            initialized: if tls_segment.file_size == 0 {
                Vec::new()
            } else {
                memory.copy(
                    elf_address(memory, load_bias + tls_segment.address)?,
                    tls_segment.file_size,
                )?
            },
            ..template.clone()
        }),
        _ => None,
    };
    for mapping in planned {
        memory.protect(
            elf_address(memory, mapping.base)?,
            mapping.byte_length,
            mapping.permissions,
        )?;
    }
    for segment in elf
        .segments
        .iter()
        .filter(|segment| segment.segment_type == 0x6474_e552)
    {
        let start = load_bias + down(segment.address);
        let end = load_bias + down(segment.address + segment.memory_size as u64);
        let mut address = start;
        while address < end {
            if !planned
                .iter()
                .any(|range| address >= range.base && address + PAGE <= range.base + range.byte_length as u64)
            {
                return Err(elf_error("RELRO exceeds this image's mapped pages"));
            }
            address += PAGE;
        }
        if end > start {
            memory.protect(
                elf_address(memory, start)?,
                checked_number(end - start, "RELRO length")?,
                GuestPermissions::Read,
            )?;
        }
    }
    let mut checked: Vec<GuestAddress> = preinitializers
        .iter()
        .chain(&initializers)
        .chain(&finalizers)
        .copied()
        .collect();
    if let Some(entry) = image.entry_point {
        checked.push(entry);
    }
    for address in checked {
        memory.check(address, 1, crate::core::contracts::GuestAccess::Execute)?;
    }
    let mappings: Vec<GuestMapping> = memory
        .mappings()
        .into_iter()
        .filter(|mapping| {
            planned.iter().any(|range| {
                mapping.base >= range.base
                    && mapping.base + mapping.byte_length as u64 <= range.base + range.byte_length as u64
            })
        })
        .collect();
    image.mappings = mappings;
    image.imports = imports;
    image.exports = exports;
    image.tls = tls;
    image.initializers = initializers.clone();
    image.finalizers = finalizers.clone();
    image.unwind = unwind;
    Ok(ElfGuestImage {
        image,
        elf: elf.clone(),
        load_bias,
        needed_libraries: elf.needed_libraries.clone(),
        soname: elf.soname.clone(),
        executable_stack: stack.map(|stack| stack.flags & 1 != 0),
        preinitializers,
        tls_exports: exported
            .iter()
            .filter(|symbol| symbol.symbol_type == 6)
            .map(|symbol| ElfTlsExport {
                symbol: symbol_name(symbol),
                offset: symbol.value,
                byte_length: symbol.size,
            })
            .collect(),
    })
}

fn pointer_array(
    elf: &ElfInspection,
    memory: &mut SparseGuestMemory,
    load_bias: u64,
    address_tag: u32,
    size_tag: u32,
) -> Result<Vec<GuestAddress>, GuestError> {
    let address = dynamic_value(&elf.dynamic, address_tag);
    let size = dynamic_value(&elf.dynamic, size_tag);
    if address.is_none() && size.is_none() {
        return Ok(Vec::new());
    }
    let (Some(address), Some(size)) = (address, size) else {
        return Err(elf_error("incomplete initializer/finalizer array"));
    };
    if size % elf.abi.pointer_bytes() as u64 != 0 {
        return Err(elf_error("incomplete initializer/finalizer array"));
    }
    let length = checked_number(size, "function array length")?;
    if length == 0 {
        return Ok(Vec::new());
    }
    elf_file_offset(&elf.segments, address, length)?;
    let bytes = memory.copy(elf_address(memory, load_bias + address)?, length)?;
    let mut result = Vec::new();
    let sentinel = if elf.abi.pointer_bytes() == 8 {
        u64::MAX
    } else {
        u64::from(u32::MAX)
    };
    let mut offset = 0;
    while offset < length {
        let value = if elf.abi.pointer_bytes() == 8 {
            let mut word = [0u8; 8];
            word.copy_from_slice(&bytes[offset..offset + 8]);
            u64::from_le_bytes(word)
        } else {
            u64::from(u32::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ]))
        };
        if value != 0 && value != sentinel {
            result.push(elf_address(memory, value)?);
        }
        offset += elf.abi.pointer_bytes();
    }
    Ok(result)
}

fn validate_dynamic_policy(elf: &ElfInspection) -> Result<(), GuestError> {
    for tag in [0x7fff_ffff, 0x7fff_fffd, 0x06ff_fefb, 0x06ff_fefc] {
        if elf.dynamic.contains_key(&tag) {
            return Err(elf_error(format!(
                "dynamic filter/audit dependency tag 0x{tag:x} is unsupported"
            )));
        }
    }
    let supported: HashSet<u32> = [
        1,
        2,
        3,
        4,
        5,
        6,
        7,
        8,
        9,
        10,
        11,
        12,
        13,
        14,
        15,
        16,
        17,
        18,
        19,
        20,
        21,
        22,
        23,
        24,
        25,
        26,
        27,
        28,
        29,
        30,
        32,
        33,
        35,
        36,
        37,
        0x6fff_fef5,
        0x6fff_fff0,
        0x6fff_fff9,
        0x6fff_fffa,
        0x6fff_fffb,
        0x6fff_fffc,
        0x6fff_fffd,
        0x6fff_fffe,
        0x6fff_ffff,
    ]
    .into_iter()
    .collect();
    for tag in elf.dynamic.keys() {
        if !supported.contains(tag) {
            return Err(elf_error(format!("unsupported dynamic tag 0x{tag:x}")));
        }
    }
    let flags = dynamic_value(&elf.dynamic, 30).unwrap_or(0);
    if flags & !31 != 0 {
        return Err(elf_error(format!("unsupported DT_FLAGS 0x{flags:x}")));
    }
    let flags1 = dynamic_value(&elf.dynamic, 0x6fff_fffb).unwrap_or(0);
    // NOW is satisfied by eager binding; PIE identifies the placement model.
    if flags1 & !(1 | 0x0800_0000) != 0 {
        return Err(elf_error(format!("unsupported DT_FLAGS_1 0x{flags1:x}")));
    }
    Ok(())
}
