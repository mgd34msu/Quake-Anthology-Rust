//! Eager ELF relocation: no lazy host PLT trampolines.
//!
//! Donor: `src/guest/elf/relocate.ts`. GNU IFUNC and defined IFUNC slots
//! resolve through explicit guest resolver execution; COPY relocations copy
//! from provider definitions outside the requesting image.

use crate::core::contracts::{
    GuestAddress, GuestImage, GuestImport, GuestImportResolution, GuestImportResolver,
    GuestSymbolName,
};
use crate::core::memory::SparseGuestMemory;
use crate::elf::parse::{dynamic_value, ElfInspection, ElfRelocation, ElfSymbol};
use crate::error::GuestError;

/// ELF TLS module: process-wide id plus its thread-pointer displacement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfTlsModule {
    /// Process-wide TLS module id (begins at one).
    pub module_id: u64,
    /// Signed displacement from the thread pointer to this module's block.
    pub thread_pointer_offset: Option<i64>,
}

/// Resolved TLS target: module plus symbol offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfTlsResolution {
    /// Process-wide TLS module id.
    pub module_id: u64,
    /// Signed thread-pointer displacement.
    pub thread_pointer_offset: Option<i64>,
    /// Symbol offset within the module block.
    pub offset: u64,
}

/// Guest thread/module TLS allocation backing TLS relocations.
pub trait ElfTlsBindings {
    /// Current module.
    fn current(&self) -> ElfTlsModule;

    /// Resolve an imported TLS symbol, or `None` for a local definition.
    fn resolve(
        &self,
        import: &GuestImport,
        requesting: &GuestImage,
    ) -> Result<Option<ElfTlsResolution>, GuestError>;
}

/// ELF relocation context: image, memory, resolver, and TLS bindings.
pub struct ElfRelocationContext<'e, 'm, 'i, 'r, 't, 'd, 's, 'u, 'p> {
    /// ELF inspection.
    pub elf: &'e ElfInspection,
    /// Guest memory.
    pub memory: &'m mut SparseGuestMemory,
    /// Requesting image.
    pub image: &'i GuestImage,
    /// Load bias added to image-relative addresses.
    pub load_bias: u64,
    /// Import resolver.
    pub resolver: &'r dyn GuestImportResolver,
    /// TLS bindings, if any.
    pub tls: Option<&'t dyn ElfTlsBindings>,
    /// Explicit guest IFUNC resolver execution.
    pub resolve_indirect:
        Option<&'d dyn Fn(GuestAddress) -> Result<GuestAddress, GuestError>>,
    /// Provider symbol-size metadata for COPY/SIZE relocations.
    pub resolve_symbol_size:
        Option<&'s dyn Fn(&GuestImport, &GuestImage) -> Result<Option<usize>, GuestError>>,
    /// Process-wide GNU unique-symbol registry.
    pub unique_symbols: Option<&'u mut std::collections::HashMap<String, GuestAddress>>,
    /// Collected imports (including TLS and COPY sources).
    pub imports: &'p mut Vec<GuestImport>,
}

fn elf_error(detail: impl Into<String>) -> GuestError {
    GuestError::bad_image("elf", detail)
}

/// Non-null guest address.
pub fn elf_address(memory: &SparseGuestMemory, value: u64) -> Result<GuestAddress, GuestError> {
    memory
        .pointer(value)?
        .ok_or_else(|| elf_error("required address is null"))
}

/// Import slot for one symbol.
#[must_use]
pub fn symbol_import(symbol: &ElfSymbol, slot: GuestAddress) -> GuestImport {
    GuestImport {
        library: symbol.version.as_ref().map_or(String::new(), |version| version.library.clone()),
        symbol: GuestSymbolName::Name {
            name: symbol.name.clone(),
            version: symbol.version.as_ref().map(|version| version.name.clone()),
        },
        slot,
        weak: symbol.binding == 2,
    }
}

/// Apply every relocation eagerly. GNU IFUNC slots defer until direct
/// relocations have landed.
pub fn relocate_elf(context: &mut ElfRelocationContext<'_, '_, '_, '_, '_, '_, '_, '_, '_>) -> Result<(), GuestError> {
    let indirect_type = if context.elf.abi.pointer_bytes() == 8 {
        37
    } else {
        42
    };
    let mut deferred: Vec<ElfRelocation> = Vec::new();
    for relocation in &context.elf.relocations {
        let symbol = context.elf.symbols.get(relocation.symbol_index);
        let indirect = relocation.relocation_type == indirect_type;
        if indirect || symbol.is_some_and(|symbol| symbol.symbol_type == 10 && symbol.section != 0) {
            deferred.push(relocation.clone());
        } else {
            apply(context, relocation)?;
        }
    }
    for relocation in &deferred {
        apply(context, relocation)?;
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn apply(context: &mut ElfRelocationContext<'_, '_, '_, '_, '_, '_, '_, '_, '_>, relocation: &ElfRelocation) -> Result<(), GuestError> {
    let wide = context.elf.abi.pointer_bytes() == 8;
    let relocation_type = relocation.relocation_type;
    if relocation_type == 0 {
        return Ok(());
    }
    let width: usize = if wide {
        if relocation_type == 12 || relocation_type == 13 {
            2
        } else if relocation_type == 14 || relocation_type == 15 {
            1
        } else if [2, 4, 10, 11, 23, 32].contains(&relocation_type) {
            4
        } else {
            8
        }
    } else {
        4
    };
    let symbol = context.elf.symbols.get(relocation.symbol_index).cloned();
    let byte_length = if relocation_type == 5 {
        symbol.as_ref().map_or(width, |symbol| symbol.size)
    } else {
        width
    };
    if !context.elf.segments.iter().any(|segment| {
        segment.segment_type == 1
            && relocation.address >= segment.address
            && relocation.address + byte_length as u64
                <= segment.address + segment.memory_size as u64
    }) {
        return Err(elf_error(format!(
            "relocation target 0x{:x} lies outside this image",
            relocation.address
        )));
    }
    let slot = elf_address(context.memory, context.load_bias.wrapping_add(relocation.address))?;
    let bytes = context.memory.copy(slot, byte_length)?;
    let addend: i64 = if relocation_type == 5 {
        0
    } else if let Some(addend) = relocation.addend {
        addend
    } else {
        match width {
            8 => {
                let mut word = [0u8; 8];
                word.copy_from_slice(&bytes[..8]);
                i64::from_le_bytes(word)
            }
            4 => {
                let mut word = [0u8; 4];
                word.copy_from_slice(&bytes[..4]);
                i64::from(i32::from_le_bytes(word))
            }
            2 => {
                let mut word = [0u8; 2];
                word.copy_from_slice(&bytes[..2]);
                i64::from(i16::from_le_bytes(word))
            }
            _ => i64::from(bytes[0] as i8),
        }
    };
    let place = slot.offset;
    let mut signed = false;
    let mut checked = wide;
    let result: i128;
    if relocation_type == 8 || (wide && relocation_type == 38) {
        if relocation.symbol_index != 0 {
            return Err(elf_error("relative relocation has a symbol"));
        }
        result = context.load_bias as i128 + addend as i128;
    } else if relocation_type == if wide { 37 } else { 42 } {
        if relocation.symbol_index != 0 || context.resolve_indirect.is_none() {
            return Err(elf_error("IRELATIVE requires explicit guest resolver execution"));
        }
        let resolver = context
            .resolve_indirect
            .ok_or_else(|| elf_error("IRELATIVE requires explicit guest resolver execution"))?;
        let address = resolver(elf_address(
            context.memory,
            context.load_bias.wrapping_add(addend as u64),
        )?)?;
        context.memory.check(address, 1, crate::core::contracts::GuestAccess::Execute)?;
        result = address.offset as i128;
    } else if wide
        && [16, 17, 18, 23].contains(&relocation_type)
        || !wide && [14, 17, 34, 35, 36, 37].contains(&relocation_type)
    {
        let Some(tls) = context.tls else {
            return Err(elf_error(format!(
                "TLS relocation {relocation_type} requires a guest thread/module allocation"
            )));
        };
        let target: ElfTlsResolution;
        if relocation.symbol_index == 0 {
            let current = tls.current();
            target = ElfTlsResolution {
                module_id: current.module_id,
                thread_pointer_offset: current.thread_pointer_offset,
                offset: 0,
            };
        } else {
            let Some(symbol) = &symbol else {
                return Err(elf_error("TLS relocation references a non-TLS symbol"));
            };
            if symbol.symbol_type != 6 {
                return Err(elf_error("TLS relocation references a non-TLS symbol"));
            }
            let import = symbol_import(symbol, slot);
            let local = symbol.section != 0
                && (symbol.binding == 0 || symbol.visibility != 0 || symbolic_binding(context.elf));
            let resolved = if local {
                None
            } else {
                tls.resolve(&import, context.image)?
            };
            if let Some(resolved) = resolved {
                target = resolved;
                context.imports.push(import);
            } else if symbol.section != 0 {
                let current = tls.current();
                target = ElfTlsResolution {
                    module_id: current.module_id,
                    thread_pointer_offset: current.thread_pointer_offset,
                    offset: symbol.value,
                };
            } else {
                return Err(elf_error(format!("unresolved TLS symbol {}", symbol.name)));
            }
        }
        if target.module_id == 0 {
            return Err(elf_error("ELF TLS module IDs begin at one"));
        }
        if relocation_type == if wide { 16 } else { 35 } {
            result = target.module_id as i128;
        } else if relocation_type == if wide { 17 } else { 36 } {
            result = target.offset as i128 + if wide { addend as i128 } else { 0 };
        } else {
            let Some(thread_pointer_offset) = target.thread_pointer_offset else {
                return Err(elf_error("static TLS relocation has no signed thread-pointer offset"));
            };
            let displacement = thread_pointer_offset as i128 + target.offset as i128;
            result = (if !wide && (relocation_type == 34 || relocation_type == 37) {
                -displacement
            } else {
                displacement
            }) + addend as i128;
            signed = true;
        }
    } else {
        let Some(symbol) = symbol else {
            return Err(elf_error(format!(
                "relocation {relocation_type} lacks symbol {}",
                relocation.symbol_index
            )));
        };
        let symbol_address = resolve_symbol(context, &symbol, slot, relocation_type == 5)?;
        if relocation_type == 5 {
            if symbol_address == 0 {
                if !symbol_import(&symbol, slot).weak {
                    return Err(elf_error("COPY source is null"));
                }
                return Ok(());
            }
            if context.image.mappings.iter().any(|mapping| {
                symbol_address >= mapping.base
                    && symbol_address < mapping.base + mapping.byte_length as u64
            }) {
                return Err(elf_error("COPY must resolve a definition outside the requesting image"));
            }
            let source_size = external_symbol_size(context, &symbol, slot)?;
            let bytes = context
                .memory
                .copy(elf_address(context.memory, symbol_address)?, source_size.min(symbol.size))?;
            context.memory.write(slot, &bytes)?;
            return Ok(());
        }
        match relocation_type {
            1 => result = symbol_address as i128 + addend as i128,
            2 => {
                result = symbol_address as i128 + addend as i128 - place as i128;
                signed = true;
            }
            4 => return Err(elf_error("PLT32 relocation requires a link-editor PLT address")),
            6 | 7 => result = symbol_address as i128,
            9 => {
                if wide {
                    return Err(elf_error("dynamic x64 GOTPCREL requires a link-editor GOT slot"));
                }
                result = symbol_address as i128 + addend as i128 - got_address(context)? as i128;
            }
            10 => {
                if wide {
                    result = symbol_address as i128 + addend as i128;
                } else {
                    result = got_address(context)? as i128 + addend as i128 - place as i128;
                }
            }
            11 => {
                if !wide {
                    return Err(elf_error("R_386_32PLT requires a link-editor PLT address"));
                }
                result = symbol_address as i128 + addend as i128;
                signed = true;
            }
            12 | 14 => {
                if !wide {
                    return Err(elf_error(format!("unsupported i386 relocation {relocation_type}")));
                }
                result = symbol_address as i128 + addend as i128;
            }
            13 | 15 | 24 => {
                if !wide {
                    return Err(elf_error(format!("unsupported i386 relocation {relocation_type}")));
                }
                result = symbol_address as i128 + addend as i128 - place as i128;
                signed = true;
            }
            32 | 33 | 38 => {
                if wide && relocation_type == 38 || !wide && relocation_type != 38 {
                    return Err(elf_error(format!("unsupported relocation {relocation_type}")));
                }
                let defined = symbol.section != 0
                    && symbol_address
                        == (if symbol.section == 0xfff1 {
                            symbol.value
                        } else {
                            context.load_bias.wrapping_add(symbol.value)
                        });
                result = (if defined {
                    symbol.size
                } else {
                    external_symbol_size(context, &symbol, slot)?
                }) as i128
                    + addend as i128;
            }
            _ => {
                return Err(elf_error(format!(
                    "unsupported {} relocation {relocation_type} for {}",
                    if wide { "x86-64" } else { "i386" },
                    symbol.name
                )));
            }
        }
    }
    // x86-32 relocation arithmetic is modulo 2^32. AMD64 narrow forms have
    // ABI overflow checks.
    if width == 8 {
        checked = false;
    }
    let bits = (width * 8) as u32;
    let (lower, upper) = if signed {
        (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
    } else {
        (0, (1i128 << bits) - 1)
    };
    if checked && (result < lower || result > upper) {
        return Err(elf_error(format!(
            "relocation {relocation_type} overflows {} {bits} bits",
            if signed { "signed" } else { "unsigned" }
        )));
    }
    let normalized = result as u64;
    let mut bytes = vec![0u8; width];
    match width {
        8 => bytes.copy_from_slice(&normalized.to_le_bytes()),
        4 => bytes.copy_from_slice(&(normalized as u32).to_le_bytes()),
        2 => bytes.copy_from_slice(&(normalized as u16).to_le_bytes()),
        _ => bytes[0] = normalized as u8,
    }
    context.memory.write(slot, &bytes)?;
    Ok(())
}

fn external_symbol_size(
    context: &mut ElfRelocationContext<'_, '_, '_, '_, '_, '_, '_, '_, '_>,
    symbol: &ElfSymbol,
    slot: GuestAddress,
) -> Result<usize, GuestError> {
    let Some(resolve) = context.resolve_symbol_size else {
        return Err(elf_error(format!(
            "COPY/SIZE relocation for {} needs provider symbol size metadata",
            symbol.name
        )));
    };
    let size = resolve(&symbol_import(symbol, slot), context.image)?;
    match size {
        Some(size) => Ok(size),
        None => Err(elf_error(format!(
            "COPY/SIZE relocation for {} needs provider symbol size metadata",
            symbol.name
        ))),
    }
}

fn got_address(context: &ElfRelocationContext<'_, '_, '_, '_, '_, '_, '_, '_, '_>) -> Result<u64, GuestError> {
    let Some(got) = dynamic_value(&context.elf.dynamic, 3) else {
        return Err(elf_error("GOT relocation lacks DT_PLTGOT"));
    };
    Ok(context.load_bias.wrapping_add(got))
}

/// Address of a defined symbol, executing GNU IFUNC resolvers explicitly.
pub fn defined_symbol_address(
    memory: &mut SparseGuestMemory,
    load_bias: u64,
    resolve_indirect: Option<&dyn Fn(GuestAddress) -> Result<GuestAddress, GuestError>>,
    symbol: &ElfSymbol,
) -> Result<u64, GuestError> {
    if symbol.section == 0xfff2 {
        return Err(elf_error(format!("unallocated COMMON symbol {}", symbol.name)));
    }
    if symbol.section >= 0xff00 && symbol.section != 0xfff1 {
        return Err(elf_error(format!(
            "unsupported reserved symbol section for {}",
            symbol.name
        )));
    }
    if symbol.symbol_type == 6 {
        return Err(elf_error(format!(
            "TLS symbol {} is an offset, not an image address",
            symbol.name
        )));
    }
    let raw = if symbol.section == 0xfff1 {
        symbol.value
    } else {
        load_bias.wrapping_add(symbol.value)
    };
    if symbol.symbol_type != 10 {
        return Ok(raw);
    }
    let Some(resolve) = resolve_indirect else {
        return Err(elf_error(format!(
            "GNU IFUNC {} requires guest resolver execution",
            symbol.name
        )));
    };
    let resolved = resolve(elf_address(memory, raw)?)?;
    memory.check(resolved, 1, crate::core::contracts::GuestAccess::Execute)?;
    Ok(resolved.offset)
}

/// Resolve one symbol through local definitions, unique symbols, and the
/// import resolver.
pub fn resolve_symbol(
    context: &mut ElfRelocationContext<'_, '_, '_, '_, '_, '_, '_, '_, '_>,
    symbol: &ElfSymbol,
    slot: GuestAddress,
    copy: bool,
) -> Result<u64, GuestError> {
    if symbol.index == 0 {
        return Ok(0);
    }
    if ![0, 1, 2, 10].contains(&symbol.binding) {
        return Err(elf_error(format!(
            "unsupported symbol binding {} for {}",
            symbol.binding, symbol.name
        )));
    }
    let own = symbol.section != 0;
    let unique = symbol.binding == 10;
    if unique && context.unique_symbols.is_none() {
        return Err(elf_error("GNU unique symbols require a process-wide symbol registry"));
    }
    if unique {
        if let Some(existing) = context
            .unique_symbols
            .as_ref()
            .and_then(|registry| registry.get(&symbol.name))
            .copied()
        {
            context.memory.check(existing, 1, crate::core::contracts::GuestAccess::Read)?;
            if !own {
                context.imports.push(symbol_import(symbol, slot));
            }
            return Ok(existing.offset);
        }
    }
    if !copy && own && (symbol.binding == 0 || symbol.visibility != 0 || symbolic_binding(context.elf))
    {
        return defined_symbol_address(
            context.memory,
            context.load_bias,
            context.resolve_indirect,
            symbol,
        );
    }
    let import = symbol_import(symbol, slot);
    let resolution = context.resolver.resolve(&import, context.image);
    if !matches!(resolution, GuestImportResolution::Unresolved { .. }) {
        let address = match &resolution {
            GuestImportResolution::Guest { address, .. }
            | GuestImportResolution::Host { address, .. } => *address,
            GuestImportResolution::Unresolved { .. } => unreachable!("checked above"),
        };
        // Every provider shares this guest address space, including host
        // callback trap slots.
        if address.space != context.memory.address_space() {
            return Err(elf_error(format!(
                "symbol {} resolves to another guest address space",
                symbol.name
            )));
        }
        context.memory.check(
            address,
            1,
            if symbol.symbol_type == 2 || symbol.symbol_type == 10 {
                crate::core::contracts::GuestAccess::Execute
            } else {
                crate::core::contracts::GuestAccess::Read
            },
        )?;
        if unique {
            if let Some(registry) = context.unique_symbols.as_deref_mut() {
                registry.insert(symbol.name.clone(), address);
            }
        }
        if !own || copy {
            context.imports.push(import);
        }
        return Ok(address.offset);
    }
    if !copy && own {
        let address = defined_symbol_address(
            context.memory,
            context.load_bias,
            context.resolve_indirect,
            symbol,
        )?;
        if unique {
            if let Some(registry) = context.unique_symbols.as_deref_mut() {
                registry.insert(symbol.name.clone(), elf_address(context.memory, address)?);
            }
        }
        return Ok(address);
    }
    let detail = match &resolution {
        GuestImportResolution::Unresolved { detail, .. } => detail.clone(),
        _ => unreachable!("checked above"),
    };
    context.imports.push(import);
    if symbol.binding == 2 {
        return Ok(0);
    }
    let version = symbol
        .version
        .as_ref()
        .map_or(String::new(), |version| format!("@{}", version.name));
    Err(elf_error(format!(
        "unresolved import {}:{}{}: {}",
        if symbol.version.as_ref().map_or("", |version| version.library.as_str()).is_empty() {
            "<global>"
        } else {
            symbol.version.as_ref().map_or("", |version| version.library.as_str())
        },
        symbol.name,
        version,
        detail
    )))
}

fn symbolic_binding(elf: &ElfInspection) -> bool {
    elf.dynamic.contains_key(&16) || dynamic_value(&elf.dynamic, 30).unwrap_or(0) & 2 != 0
}
