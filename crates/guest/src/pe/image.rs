//! PE load image: staged RVA reads over the mapped image bytes.
//!
//! Donor: `src/guest/pe/image.ts`. Every read validates its RVA against the
//! headers and section table; executable RVAs additionally require an
//! executable section.

use qa_core::identity::ProviderId;

use crate::core::contracts::{GuestAddress, GuestImage, ModuleIdentity, NativeAbi};
use crate::core::memory::SparseGuestMemory;
use crate::error::GuestError;
use crate::pe::format::{checked_range, pe_error, PeFile, PeStage};

/// One x64 unwind record with its exact metadata bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeUnwindRecord {
    /// Function begin RVA.
    pub begin_rva: u32,
    /// Function end RVA.
    pub end_rva: u32,
    /// Unwind-info RVA.
    pub unwind_info_rva: u32,
    /// Unwind version.
    pub version: u32,
    /// Unwind flags.
    pub flags: u32,
    /// Handler RVA, if any.
    pub handler_rva: Option<u32>,
    /// Handler-data RVA, if any.
    pub handler_data_rva: Option<u32>,
    /// Chained record, if any.
    pub chained: Option<Box<PeUnwindRecord>>,
    /// Exact UNWIND_INFO header/codes plus handler RVA or chained function.
    pub metadata: Vec<u8>,
}

/// Load-configuration directory: raw bytes plus security slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeLoadConfiguration {
    /// Directory address.
    pub address: GuestAddress,
    /// Raw directory bytes.
    pub bytes: Vec<u8>,
    /// Security-cookie slot, if any.
    pub security_cookie_address: Option<GuestAddress>,
    /// Guard-check slot, if any.
    pub guard_check_slot: Option<GuestAddress>,
    /// Guard-dispatch slot, if any.
    pub guard_dispatch_slot: Option<GuestAddress>,
    /// Guard flags.
    pub guard_flags: u32,
}

/// Mapped PE image: base image plus PE metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeImage {
    /// Base guest image.
    pub image: GuestImage,
    /// Parsed headers.
    pub pe: PeFile,
    /// TLS index slot, if any.
    pub tls_index_address: Option<GuestAddress>,
    /// Load configuration, if any.
    pub load_configuration: Option<PeLoadConfiguration>,
    /// Unwind records.
    pub unwind_records: Vec<PeUnwindRecord>,
}

impl PeImage {
    /// Image ABI.
    #[must_use]
    pub const fn abi(&self) -> NativeAbi {
        self.image.abi
    }

    /// Owning module id.
    #[must_use]
    pub fn module_id(&self) -> &ProviderId {
        &self.image.module.id
    }
}

/// Staged reader over the mapped image bytes plus guest memory.
pub struct ImageReader<'a> {
    /// Mapped image bytes (relocated in place).
    pub bytes: Vec<u8>,
    /// Parsed headers.
    pub pe: PeFile,
    /// Load base.
    pub base: u64,
    /// Guest memory.
    pub memory: &'a mut SparseGuestMemory,
    /// Owning module.
    pub module: ModuleIdentity,
    /// Error stage.
    pub stage: PeStage,
}

impl<'a> ImageReader<'a> {
    /// Reader over `bytes` staged for `stage`.
    #[must_use]
    pub fn new(
        bytes: Vec<u8>,
        pe: PeFile,
        base: u64,
        memory: &'a mut SparseGuestMemory,
        module: ModuleIdentity,
        stage: PeStage,
    ) -> Self {
        Self {
            bytes,
            pe,
            base,
            memory,
            module,
            stage,
        }
    }

    /// Re-stage the reader in place, sharing the byte image and memory.
    pub fn for_stage(&mut self, stage: PeStage) -> &mut Self {
        self.stage = stage;
        self
    }

    /// Check that an RVA range lies within headers/sections without gaps.
    pub fn range(&self, rva: u32, size: usize) -> Result<(), GuestError> {
        checked_range(rva as usize, size, self.bytes.len(), self.stage)?;
        if size == 0 {
            return Ok(());
        }
        let mut at = rva;
        if at < self.pe.header_size {
            at = (rva as usize + size).min(self.pe.header_size as usize) as u32;
        }
        for section in &self.pe.sections {
            if at >= section.rva && at < section.rva + section.mapped_size {
                at = ((rva as usize + size) as u32).min(section.rva + section.mapped_size);
            }
            if at as usize == rva as usize + size {
                return Ok(());
            }
        }
        if at as usize != rva as usize + size {
            return Err(pe_error(
                self.stage,
                format!("RVA 0x{rva:x} crosses an unmapped image gap"),
            ));
        }
        Ok(())
    }

    /// Resolve an RVA to a guest address.
    pub fn address(&mut self, rva: u32, size: usize) -> Result<GuestAddress, GuestError> {
        self.range(rva, size)?;
        self.memory
            .pointer(self.base + u64::from(rva))?
            .ok_or_else(|| pe_error(self.stage, "null image address"))
    }

    /// Convert a VA to an RVA.
    pub fn rva(&self, va: u64, size: usize) -> Result<u32, GuestError> {
        if va < self.base || va - self.base > u64::from(self.pe.image_size) {
            return Err(pe_error(self.stage, format!("VA 0x{va:x} is outside the image")));
        }
        let rva = (va - self.base) as u32;
        self.range(rva, size)?;
        Ok(rva)
    }

    /// Resolve an RVA in an executable section.
    pub fn executable(&mut self, rva: u32) -> Result<GuestAddress, GuestError> {
        let executable = self.pe.sections.iter().any(|section| {
            rva >= section.rva
                && rva < section.rva + section.mapped_size
                && section.permissions.allows(crate::core::contracts::GuestAccess::Execute)
        });
        if !executable {
            return Err(pe_error(self.stage, format!("RVA 0x{rva:x} is not executable")));
        }
        self.address(rva, 1)
    }

    /// Checked `u16` image load.
    pub fn u16(&self, rva: u32) -> Result<u16, GuestError> {
        self.range(rva, 2)?;
        Ok(u16::from_le_bytes([
            self.bytes[rva as usize],
            self.bytes[rva as usize + 1],
        ]))
    }

    /// Checked `u32` image load.
    pub fn u32(&self, rva: u32) -> Result<u32, GuestError> {
        self.range(rva, 4)?;
        Ok(u32::from_le_bytes([
            self.bytes[rva as usize],
            self.bytes[rva as usize + 1],
            self.bytes[rva as usize + 2],
            self.bytes[rva as usize + 3],
        ]))
    }

    /// Checked pointer-width image load.
    pub fn pointer(&self, rva: u32) -> Result<u64, GuestError> {
        self.range(rva, self.pe.abi.pointer_bytes())?;
        if self.pe.abi.pointer_bytes() == 4 {
            Ok(u64::from(self.u32(rva)?))
        } else {
            let mut word = [0u8; 8];
            word.copy_from_slice(&self.bytes[rva as usize..rva as usize + 8]);
            Ok(u64::from_le_bytes(word))
        }
    }

    /// Checked ASCII string bounded by its section (or the headers).
    pub fn text(&self, rva: u32, end: u32) -> Result<String, GuestError> {
        self.range(rva, 1)?;
        let bound = self
            .pe
            .sections
            .iter()
            .find(|section| rva >= section.rva && rva < section.rva + section.mapped_size)
            .map_or(self.pe.header_size, |section| section.rva + section.mapped_size)
            .min(end);
        if bound < rva {
            return Err(pe_error(self.stage, "string bound precedes RVA"));
        }
        crate::pe::format::PeReader::new(&self.bytes, self.stage).text(rva as usize, (bound - rva) as usize)
    }

    /// Copy image bytes.
    pub fn copy(&self, rva: u32, size: usize) -> Result<Vec<u8>, GuestError> {
        self.range(rva, size)?;
        Ok(self.bytes[rva as usize..rva as usize + size].to_vec())
    }
}
