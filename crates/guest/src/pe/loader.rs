//! PE image mapping and import binding.
//!
//! Donor: `src/guest/pe/loader.ts`. Maps bytes and lifecycle metadata only;
//! imports bind through [`bind_pe_imports`] with rollback on failure.

use crate::core::contracts::{
    GuestAccess, GuestAddress, GuestImage, GuestImportResolution, GuestImportResolver, GuestMapping,
    GuestUnwindFormat, GuestUnwindRegion, ModuleIdentity,
};
use crate::core::memory::SparseGuestMemory;
use crate::error::GuestError;
use crate::pe::directories::{
    read_exports, read_imports, read_load_configuration, read_tls, read_unwind,
};
use crate::pe::format::{directory, parse_pe, pe_error, PeFile, PeReader, PeStage};
use crate::pe::image::{ImageReader, PeImage};

/// PE mapping options.
pub struct MapPeImageOptions<'a> {
    /// Image bytes.
    pub bytes: &'a [u8],
    /// Guest memory.
    pub memory: &'a mut SparseGuestMemory,
    /// Owning module (defaults to the memory owner).
    pub module: Option<ModuleIdentity>,
    /// Load base (defaults to the preferred base).
    pub base: Option<u64>,
    /// Maximum image bytes (defaults to 256 MiB).
    pub maximum_image_bytes: Option<usize>,
}

struct Patch {
    rva: u32,
    width: usize,
    value: i128,
}

/// Apply base relocations to the staged image bytes.
pub fn relocate_pe(image: &mut ImageReader) -> Result<(), GuestError> {
    let reader = image.for_stage(PeStage::Relocation);
    let table = directory(&reader.pe, 5)?;
    let delta = reader.base as i128 - reader.pe.preferred_base as i128;
    if delta != 0 && (reader.pe.characteristics & 1 != 0 || table.rva == 0) {
        return Err(pe_error(
            PeStage::Relocation,
            "image cannot be relocated away from its preferred base",
        ));
    }
    if table.rva == 0 {
        return Ok(());
    }
    let source_bytes = reader.copy(table.rva, table.byte_length as usize)?;
    let source = PeReader::new(&source_bytes, PeStage::Relocation);
    let mut patches: Vec<Patch> = Vec::new();
    let mut block = 0usize;
    while block < table.byte_length as usize {
        let page = source.u32(block)?;
        let size = source.u32(block + 4)? as usize;
        if page % 4096 != 0 || size < 8 || size % 2 != 0 || size > table.byte_length as usize - block
        {
            return Err(pe_error(
                PeStage::Relocation,
                "invalid relocation block size/page",
            ));
        }
        let mut at = block + 8;
        while at < block + size {
            let entry = source.u16(at)?;
            let relocation_type = entry >> 12;
            if relocation_type != 0 {
                let rva = page + u32::from(entry & 0xfff);
                if relocation_type == 10 && reader.pe.abi.pointer_bytes() == 8 {
                    reader.range(rva, 8)?;
                    let mut word = [0u8; 8];
                    word.copy_from_slice(&reader.bytes[rva as usize..rva as usize + 8]);
                    patches.push(Patch {
                        rva,
                        width: 8,
                        value: u64::from_le_bytes(word) as i128 + delta,
                    });
                } else if reader.pe.abi.pointer_bytes() == 4 && relocation_type == 3 {
                    patches.push(Patch {
                        rva,
                        width: 4,
                        value: reader.u32(rva)? as i128 + delta,
                    });
                } else if reader.pe.abi.pointer_bytes() == 4
                    && (relocation_type == 1 || relocation_type == 2)
                {
                    patches.push(Patch {
                        rva,
                        width: 2,
                        value: reader.u16(rva)? as i128
                            + if relocation_type == 1 { delta >> 16 } else { delta },
                    });
                } else if reader.pe.abi.pointer_bytes() == 4 && relocation_type == 4 {
                    if at + 2 >= block + size {
                        return Err(pe_error(
                            PeStage::Relocation,
                            "HIGHADJ lacks its signed low word",
                        ));
                    }
                    at += 2;
                    let low = source.u16(at)? as i16 as i128;
                    patches.push(Patch {
                        rva,
                        width: 2,
                        value: (((reader.u16(rva)? as i128) << 16) + low + delta + 0x8000) >> 16,
                    });
                } else {
                    return Err(pe_error(
                        PeStage::Relocation,
                        format!(
                            "unsupported base relocation type {relocation_type} for {}",
                            reader.pe.abi.image()
                        ),
                    ));
                }
            }
            at += 2;
        }
        block += size;
    }
    patches.sort_by_key(|patch| patch.rva);
    let mut end: i64 = -1;
    for patch in patches {
        if (patch.rva as i64) < end {
            return Err(pe_error(PeStage::Relocation, "overlapping relocation targets"));
        }
        end = patch.rva as i64 + patch.width as i64;
        let mask = if patch.width == 8 {
            u64::MAX as i128
        } else if patch.width == 4 {
            0xffff_ffffi128
        } else {
            0xffffi128
        };
        let value = (patch.value & mask) as u64;
        let at = patch.rva as usize;
        match patch.width {
            2 => reader.bytes[at..at + 2].copy_from_slice(&(value as u16).to_le_bytes()),
            4 => reader.bytes[at..at + 4].copy_from_slice(&(value as u32).to_le_bytes()),
            _ => reader.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes()),
        }
    }
    Ok(())
}

/// Map bytes and lifecycle metadata only. Bind imports and prepare
/// TLS/runtime services before execution.
pub fn map_pe_image(options: MapPeImageOptions) -> Result<PeImage, GuestError> {
    let pe: PeFile = parse_pe(options.bytes)?;
    let memory = options.memory;
    let module = options.module.unwrap_or_else(|| memory.module().clone());
    let base = options.base.unwrap_or(pe.preferred_base);
    let maximum = options.maximum_image_bytes.unwrap_or(256 * 1024 * 1024);
    if maximum == 0 || pe.image_size as usize > maximum {
        return Err(pe_error(PeStage::Mapping, "image exceeds the explicit allocation limit"));
    }
    if memory.pointer_bytes() != pe.abi.pointer_bytes()
        || base == 0
        || base % 65536 != 0
        || base as u128 + u64::from(pe.image_size) as u128 > 1u128 << (pe.abi.pointer_bytes() * 8)
    {
        return Err(pe_error(
            PeStage::Mapping,
            "incompatible guest pointer width or image base",
        ));
    }
    if directory(&pe, 14)?.rva != 0 {
        return Err(pe_error(
            PeStage::Mapping,
            "managed CLR images require an unsupported runtime",
        ));
    }
    let mut bytes = vec![0u8; pe.image_size as usize];
    bytes[..pe.header_size as usize]
        .copy_from_slice(&options.bytes[..pe.header_size as usize]);
    for section in &pe.sections {
        bytes[section.rva as usize..section.rva as usize + section.raw_size as usize].copy_from_slice(
            &options.bytes
                [section.raw_offset as usize..section.raw_offset as usize + section.raw_size as usize],
        );
    }
    let mut reader = ImageReader::new(bytes, pe.clone(), base, memory, module.clone(), PeStage::Mapping);
    for index in 0..pe.directories.len() {
        let entry = directory(&pe, index)?;
        if index != 4 && entry.rva != 0 {
            reader.range(entry.rva, entry.byte_length as usize)?;
        }
    }
    relocate_pe(&mut reader)?;
    let imports = read_imports(&mut reader)?;
    let exports = read_exports(&mut reader)?;
    let (tls, tls_index) = read_tls(&mut reader)?;
    let unwind_records = read_unwind(&mut reader)?;
    let load_configuration = read_load_configuration(&mut reader)?;
    let entry_point = if pe.entry_point_rva == 0 {
        None
    } else {
        Some(reader.executable(pe.entry_point_rva)?)
    };
    let address = reader.address(0, 1)?;
    let header_mapped_size =
        (pe.header_size as usize).next_multiple_of(pe.section_alignment as usize);
    let mut image_mappings = vec![GuestMapping {
        base,
        byte_length: header_mapped_size,
        permissions: crate::core::contracts::GuestPermissions::Read,
        label: format!("{:?}:PE headers", module.id),
    }];
    for section in &pe.sections {
        if section.mapped_size > 0 {
            image_mappings.push(GuestMapping {
                base: base + u64::from(section.rva),
                byte_length: section.mapped_size as usize,
                permissions: section.permissions,
                label: format!("{:?}:{}", module.id, section.name),
            });
        }
    }
    let staged = reader.bytes.clone();
    let memory = reader.memory;
    memory.map(&crate::core::contracts::GuestMapOptions {
        base,
        byte_length: pe.image_size as usize,
        permissions: crate::core::contracts::GuestPermissions::ReadWrite,
        label: format!("{:?}:PE image", module.id),
        bytes: Some(staged),
    })?;
    let rollback = (|| -> Result<(), GuestError> {
        memory.protect(address, pe.image_size as usize, crate::core::contracts::GuestPermissions::None)?;
        for mapping in &image_mappings {
            let at = memory.offset(address, (mapping.base - base) as i64)?;
            memory.protect(at, mapping.byte_length, mapping.permissions)?;
        }
        Ok(())
    })();
    if let Err(error) = rollback {
        memory.unmap(address, pe.image_size as usize)?;
        return Err(error);
    }
    let mut unwind = Vec::with_capacity(unwind_records.len());
    for record in &unwind_records {
        let start = memory
            .pointer(base + u64::from(record.begin_rva))?
            .ok_or_else(|| pe_error(PeStage::Mapping, "null unwind address"))?;
        let end = memory
            .pointer(base + u64::from(record.end_rva))?
            .ok_or_else(|| pe_error(PeStage::Mapping, "null unwind address"))?;
        unwind.push(GuestUnwindRegion {
            start,
            end,
            format: GuestUnwindFormat::PeX64Unwind,
            metadata: record.metadata.clone(),
        });
    }
    let mappings: Vec<GuestMapping> = memory
        .mappings()
        .into_iter()
        .filter(|mapping| mapping.base >= base && mapping.base < base + u64::from(pe.image_size))
        .collect();
    Ok(PeImage {
        image: GuestImage {
            module,
            abi: pe.abi,
            base: address,
            preferred_base: pe.preferred_base,
            byte_length: u64::from(pe.image_size),
            entry_point,
            mappings,
            imports,
            exports,
            tls,
            initializers: Vec::new(),
            finalizers: Vec::new(),
            unwind,
        },
        pe,
        tls_index_address: tls_index,
        load_configuration,
        unwind_records,
    })
}

/// Bind imports through `resolver`. Failures leave all IAT bytes untouched.
pub fn bind_pe_imports(
    image: &PeImage,
    memory: &mut SparseGuestMemory,
    resolver: &dyn GuestImportResolver,
) -> Result<Vec<GuestImportResolution>, GuestError> {
    if memory.address_space() != image.image.base.space
        || memory.pointer_bytes() != image.image.abi.pointer_bytes()
    {
        return Err(pe_error(PeStage::Imports, "image belongs to another guest address space"));
    }
    let mut resolutions = Vec::new();
    let mut writes: Vec<(GuestAddress, Vec<u8>, Vec<u8>)> = Vec::new();
    for import in &image.image.imports {
        let resolution = resolver.resolve(memory, import, &image.image);
        match &resolution {
            GuestImportResolution::Unresolved { detail, .. } => {
                return Err(pe_error(
                    PeStage::Imports,
                    format!("{}: {detail}", import.library),
                ));
            }
            GuestImportResolution::Guest { address, .. }
            | GuestImportResolution::Host { address, .. } => {
                let address = *address;
            if address.space != memory.address_space()
                || address.offset == 0
                || address.offset >= (1u128 << (memory.pointer_bytes() * 8)) as u64
                || !memory.mappings().iter().any(|mapping| {
                    address.offset >= mapping.base
                        && address.offset < mapping.base + mapping.byte_length as u64
                        && mapping.permissions != crate::core::contracts::GuestPermissions::None
                })
            {
                return Err(pe_error(
                    PeStage::Imports,
                    "resolved symbol is not a mapped address in this guest space",
                ));
            }
            let mut bytes = vec![0u8; memory.pointer_bytes()];
            if memory.pointer_bytes() == 4 {
                bytes.copy_from_slice(&(address.offset as u32).to_le_bytes());
            } else {
                bytes.copy_from_slice(&address.offset.to_le_bytes());
            }
            let previous = memory.copy(import.slot, memory.pointer_bytes())?;
            writes.push((import.slot, bytes, previous));
            resolutions.push(resolution);
            }
        }
    }
    let regions: Vec<GuestMapping> = memory
        .mappings()
        .into_iter()
        .filter(|mapping| {
            writes.iter().any(|(slot, _, _)| {
                slot.offset < mapping.base + mapping.byte_length as u64
                    && slot.offset + memory.pointer_bytes() as u64 > mapping.base
            })
        })
        .collect();
    let mut protected: Vec<GuestMapping> = Vec::new();
    let outcome = (|| -> Result<(), GuestError> {
        for region in &regions {
            let address = memory
                .pointer(region.base)?
                .ok_or_else(|| pe_error(PeStage::Imports, "null IAT region"))?;
            memory.protect(
                address,
                region.byte_length,
                if region.permissions.allows(GuestAccess::Execute) {
                    crate::core::contracts::GuestPermissions::ReadWriteExecute
                } else {
                    crate::core::contracts::GuestPermissions::ReadWrite
                },
            )?;
            protected.push(region.clone());
        }
        for (slot, bytes, _) in &writes {
            if let Err(error) = memory.write(*slot, bytes) {
                for (slot, _, previous) in &writes {
                    memory.write(*slot, previous)?;
                }
                return Err(error);
            }
        }
        Ok(())
    })();
    for region in &protected {
        let address = memory
            .pointer(region.base)?
            .ok_or_else(|| pe_error(PeStage::Imports, "null IAT protection address"))?;
        memory.protect(address, region.byte_length, region.permissions)?;
    }
    outcome?;
    Ok(resolutions)
}
