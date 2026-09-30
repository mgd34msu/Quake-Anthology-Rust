//! Port of `src/compat/q2/native-mod-region.ts`.
//! Bridges one qualified native region: runs it inside its original
//! prologue/epilogue with declared machine inputs, scoped to one invocation.

use std::collections::HashMap;
use std::rc::Rc;

use qa_guest::abi::values::{decode_value, encode_value};
use qa_guest::core::contracts::{GuestAccess, GuestAddress, GuestCallValue, GuestStorage, GuestValueLayout};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::error::GuestError;
use thiserror::Error;

/// Failures validating or executing a native region.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RegionError {
    /// Boundary, frame, or storage declaration is invalid.
    #[error("invalid standalone native region frame: {0}")]
    InvalidFrame(String),
    /// Two inputs overlap the same machine storage.
    #[error("native region inputs overlap")]
    OverlappingInputs,
    /// A register or SIMD lane exceeds the source architecture.
    #[error("native region location exceeds the source architecture: {0}")]
    BadLocation(String),
    /// Input count differs from the declaration.
    #[error("native region input count differs from its declaration")]
    InputArity,
    /// The stack left the declared frame.
    #[error("native donor {0} differs from its declared stack frame")]
    FrameMismatch(&'static str),
    /// The region completed out of order.
    #[error("native donor did not complete its declared region")]
    Incomplete,
    /// The owning authority retired.
    #[error("native donor owner retired")]
    Retired,
    /// Underlying guest failure.
    #[error("native region guest failure: {0}")]
    Guest(String),
}

impl From<GuestError> for RegionError {
    fn from(error: GuestError) -> Self {
        Self::Guest(error.to_string())
    }
}

/// Liveness authority for one region execution.
#[derive(Clone)]
pub struct RegionAuthority {
    current: Rc<dyn Fn() -> bool>,
}

impl RegionAuthority {
    /// Build an authority from a liveness predicate.
    pub fn new(current: impl Fn() -> bool + 'static) -> Self {
        Self {
            current: Rc::new(current),
        }
    }

    /// Whether the region still owns its invocation.
    #[must_use]
    pub fn is_current(&self) -> bool {
        (self.current)()
    }
}

impl std::fmt::Debug for RegionAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegionAuthority").finish_non_exhaustive()
    }
}

/// Machine location of one region input or the region result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegionLocation {
    /// General-purpose register.
    Register {
        /// Register name (`rax`, `r8`, ...).
        register: String,
        /// Lane storage.
        storage: GuestStorage,
    },
    /// SIMD lane slice.
    Simd {
        /// XMM register index.
        index: usize,
        /// Byte offset inside the register.
        offset: usize,
        /// Lane storage.
        storage: GuestStorage,
    },
    /// Stack frame slot.
    Stack {
        /// Byte offset from the frame base.
        offset: usize,
        /// Lane storage.
        storage: GuestStorage,
    },
}

impl RegionLocation {
    fn storage(&self) -> GuestStorage {
        match self {
            Self::Register { storage, .. } | Self::Simd { storage, .. } | Self::Stack { storage, .. } => *storage,
        }
    }
}

/// Original prologue/epilogue frame surrounding a region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionFrame {
    /// Prologue entry RVA.
    pub entry: u64,
    /// Epilogue exit RVA.
    pub exit: u64,
    /// Prologue stack reservation in bytes.
    pub stack_bytes: usize,
    /// Declared outgoing argument area in bytes.
    pub argument_bytes: usize,
}

/// One declared region input value.
#[derive(Debug, Clone, PartialEq)]
pub struct RegionInput {
    /// Machine target.
    pub target: RegionLocation,
    /// Value staged into the target.
    pub value: GuestCallValue,
}

/// Standalone region declaration: boundaries plus machine inputs and result.
#[derive(Debug, Clone, PartialEq)]
pub struct ProtectionRegion {
    /// Surrounding original frame.
    pub frame: RegionFrame,
    /// Region entry RVA.
    pub entry: u64,
    /// Region join RVA.
    pub join: u64,
    /// Declared inputs.
    pub inputs: Vec<RegionInput>,
    /// Result location.
    pub result: RegionLocation,
    /// Exclusion regions on the wrapped call; standalone regions forbid them.
    pub call_skips: usize,
}

struct Extent {
    bank: ExtentBank,
    offset: usize,
    length: usize,
}

#[derive(Debug, PartialEq, Eq)]
enum ExtentBank {
    Register(String),
    Simd,
    Stack,
}

fn extent(location: &RegionLocation, frame: &RegionFrame, pointer_bytes: usize) -> Result<Extent, RegionError> {
    let length = location.storage().byte_length(pointer_bytes);
    match location {
        RegionLocation::Register { register, .. } => {
            if pointer_bytes == 4 && (length > 4 || is_extended_register(register)) {
                return Err(RegionError::BadLocation(register.clone()));
            }
            Ok(Extent {
                bank: ExtentBank::Register(register.clone()),
                offset: 0,
                length,
            })
        }
        RegionLocation::Simd { index, offset, .. } => {
            let lanes = if pointer_bytes == 4 { 8 } else { 16 };
            if *index >= lanes || offset.saturating_add(length) > 16 {
                return Err(RegionError::BadLocation(format!("xmm{index}+{offset}")));
            }
            Ok(Extent {
                bank: ExtentBank::Simd,
                offset: index * 16 + offset,
                length,
            })
        }
        RegionLocation::Stack { offset, .. } => {
            let frame_end = frame
                .stack_bytes
                .saturating_add(pointer_bytes)
                .saturating_add(frame.argument_bytes);
            if offset.saturating_add(length) > frame_end
                || (*offset < frame.stack_bytes + pointer_bytes && offset.saturating_add(length) > frame.stack_bytes)
            {
                return Err(RegionError::BadLocation(format!("stack+{offset}")));
            }
            Ok(Extent {
                bank: ExtentBank::Stack,
                offset: *offset,
                length,
            })
        }
    }
}

fn is_extended_register(name: &str) -> bool {
    let rest = name.strip_prefix('r').unwrap_or("");
    matches!(rest, "8" | "9" | "10" | "11" | "12" | "13" | "14" | "15")
}

/// Validate a standalone region declaration against a pointer width.
pub fn validate_native_mod_region(region: &ProtectionRegion, pointer_bytes: usize) -> Result<(), RegionError> {
    if pointer_bytes != 4 && pointer_bytes != 8 {
        return Err(RegionError::InvalidFrame("pointer width".to_string()));
    }
    let boundaries = [region.frame.entry, region.entry, region.join, region.frame.exit];
    let distinct = {
        let mut sorted = boundaries;
        sorted.sort_unstable();
        sorted.windows(2).all(|pair| pair[0] != pair[1])
    };
    if !distinct
        || [region.frame.stack_bytes, region.frame.argument_bytes]
            .iter()
            .any(|value| value % pointer_bytes != 0)
        || region.call_skips != 0
        || region.result.storage() == GuestStorage::Pointer
    {
        return Err(RegionError::InvalidFrame("boundaries".to_string()));
    }
    let mut ranges: Vec<Extent> = Vec::with_capacity(region.inputs.len());
    for input in &region.inputs {
        let range = extent(&input.target, &region.frame, pointer_bytes)?;
        if ranges.iter().any(|previous| {
            previous.bank == range.bank
                && range.offset < previous.offset + previous.length
                && previous.offset < range.offset + range.length
        }) {
            return Err(RegionError::OverlappingInputs);
        }
        ranges.push(range);
    }
    extent(&region.result, &region.frame, pointer_bytes)?;
    Ok(())
}

/// Headless region host: guest memory plus synthetic registers and stack.
pub struct SyntheticRegionHost {
    /// Guest memory backing the image and the stack.
    pub memory: SparseGuestMemory,
    /// Image base address.
    pub image_base: GuestAddress,
    registers: HashMap<String, u64>,
    xmm: Vec<u8>,
    rsp: u64,
}

impl SyntheticRegionHost {
    /// Build a host over `memory` with the stack pointer at `rsp`.
    pub fn new(memory: SparseGuestMemory, image_base: GuestAddress, rsp: u64) -> Self {
        Self {
            memory,
            image_base,
            registers: HashMap::new(),
            xmm: vec![0; 256],
            rsp,
        }
    }

    /// Current stack pointer.
    #[must_use]
    pub fn stack_pointer(&self) -> u64 {
        self.rsp
    }

    /// Move the stack pointer (prologue reservation).
    pub fn set_stack_pointer(&mut self, rsp: u64) {
        self.rsp = rsp;
    }

    /// Read a general-purpose register.
    #[must_use]
    pub fn read_register(&self, name: &str) -> u64 {
        self.registers.get(name).copied().unwrap_or(0)
    }

    fn at(&self, rva: u64) -> Result<GuestAddress, GuestError> {
        self.memory
            .offset(self.image_base, i64::try_from(rva).unwrap_or(i64::MAX))
    }

    fn stack_address(&self, offset: usize) -> GuestAddress {
        GuestAddress::new(self.memory.address_space(), self.rsp.wrapping_add(offset as u64))
    }

    fn read_location(&mut self, location: &RegionLocation) -> Result<Vec<u8>, RegionError> {
        let pointer_bytes = self.memory.pointer_bytes();
        let count = location.storage().byte_length(pointer_bytes);
        match location {
            RegionLocation::Stack { offset, .. } => {
                let address = self.stack_address(*offset);
                Ok(self.memory.copy(address, count)?)
            }
            RegionLocation::Simd { index, offset, .. } => {
                let start = index * 16 + offset;
                Ok(self.xmm[start..start + count].to_vec())
            }
            RegionLocation::Register { register, .. } => {
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(&self.read_register(register).to_le_bytes());
                Ok(bytes[..count].to_vec())
            }
        }
    }

    fn write_location(
        &mut self,
        location: &RegionLocation,
        value: &GuestCallValue,
        saved_stack: &mut Vec<(GuestAddress, Vec<u8>)>,
    ) -> Result<(), RegionError> {
        let layout = GuestValueLayout::Scalar(location.storage());
        let bytes = encode_value(&layout, value, &self.memory)?;
        match location {
            RegionLocation::Stack { offset, .. } => {
                let address = self.stack_address(*offset);
                saved_stack.push((address, self.memory.copy(address, bytes.len())?));
                self.memory.write(address, &bytes)?;
            }
            RegionLocation::Simd { index, offset, .. } => {
                let start = index * 16 + offset;
                self.xmm[start..start + bytes.len()].copy_from_slice(&bytes);
            }
            RegionLocation::Register { register, .. } => {
                let mut raw = [0u8; 8];
                raw[..bytes.len()].copy_from_slice(&bytes);
                self.registers.insert(register.clone(), u64::from_le_bytes(raw));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Prologue,
    Region,
    Executing,
    Epilogue,
}

#[derive(Debug, Clone)]
struct ProcessorSnapshot {
    registers: HashMap<String, u64>,
    xmm: Vec<u8>,
}

/// One scoped execution of a declared region.
pub struct NativeModRegionExecution {
    definition: ProtectionRegion,
    inputs: Vec<GuestCallValue>,
    authority: Option<RegionAuthority>,
    entry_stack: Option<u64>,
    before: Option<ProcessorSnapshot>,
    phase: Phase,
    output: Option<GuestCallValue>,
    saved_stack: Vec<(GuestAddress, Vec<u8>)>,
}

impl NativeModRegionExecution {
    /// Bind the declaration, checking input arity and execute permission.
    pub fn new(
        host: &mut SyntheticRegionHost,
        definition: ProtectionRegion,
        inputs: Vec<GuestCallValue>,
        authority: Option<RegionAuthority>,
    ) -> Result<Self, RegionError> {
        validate_native_mod_region(&definition, host.memory.pointer_bytes())?;
        if inputs.len() != definition.inputs.len() {
            return Err(RegionError::InputArity);
        }
        for rva in [
            definition.frame.entry,
            definition.entry,
            definition.join,
            definition.frame.exit,
        ] {
            let address = host.at(rva)?;
            host.memory.check(address, 1, GuestAccess::Execute)?;
        }
        Ok(Self {
            definition,
            inputs,
            authority,
            entry_stack: None,
            before: None,
            phase: Phase::Prologue,
            output: None,
            saved_stack: Vec::new(),
        })
    }

    fn assert_current(&self) -> Result<(), RegionError> {
        if self.authority.as_ref().is_some_and(|authority| !authority.is_current()) {
            return Err(RegionError::Retired);
        }
        Ok(())
    }

    fn in_frame(&self, host: &SyntheticRegionHost) -> bool {
        self.entry_stack
            .is_some_and(|entry| host.stack_pointer() == entry.wrapping_sub(self.definition.frame.stack_bytes as u64))
    }

    /// Observe target entry: records the entry stack once.
    pub fn observe_entry(&mut self, host: &SyntheticRegionHost) {
        if self.entry_stack.is_none() {
            self.entry_stack = Some(host.stack_pointer());
        }
    }

    /// Run the prologue hook: check the frame, snapshot, stage inputs, skip.
    pub fn run_prologue(&mut self, host: &mut SyntheticRegionHost) -> Result<(), RegionError> {
        self.assert_current()?;
        if self.phase != Phase::Prologue || !self.in_frame(host) {
            return Err(RegionError::FrameMismatch("prologue"));
        }
        self.before = Some(ProcessorSnapshot {
            registers: host.registers.clone(),
            xmm: host.xmm.clone(),
        });
        for (index, input) in self.definition.inputs.clone().iter().enumerate() {
            let value = self.inputs.get(index).ok_or(RegionError::InputArity)?;
            host.write_location(&input.target, value, &mut self.saved_stack)?;
        }
        self.phase = Phase::Region;
        Ok(())
    }

    /// Run the region body, capture the result, then restore staged state.
    pub fn run_region(
        &mut self,
        host: &mut SyntheticRegionHost,
        body: impl FnOnce(&mut SyntheticRegionHost),
    ) -> Result<(), RegionError> {
        self.assert_current()?;
        if self.phase != Phase::Region || !self.in_frame(host) {
            return Err(RegionError::FrameMismatch("region"));
        }
        self.phase = Phase::Executing;
        body(host);
        self.assert_current()?;
        let bytes = host.read_location(&self.definition.result.clone())?;
        let layout = GuestValueLayout::Scalar(self.definition.result.storage());
        self.output = Some(decode_value(&layout, &bytes, &host.memory)?);
        let Some(before) = self.before.clone() else {
            return Err(RegionError::Incomplete);
        };
        for (address, bytes) in self.saved_stack.drain(..).rev() {
            host.memory.write(address, &bytes)?;
        }
        host.registers = before.registers;
        host.xmm = before.xmm;
        self.phase = Phase::Epilogue;
        Ok(())
    }

    /// Run the epilogue hook: check the frame, skip to the exit.
    pub fn run_epilogue(&mut self, host: &SyntheticRegionHost) -> Result<(), RegionError> {
        self.assert_current()?;
        if self.phase != Phase::Epilogue || !self.in_frame(host) {
            return Err(RegionError::FrameMismatch("epilogue"));
        }
        Ok(())
    }

    /// Take the captured region result.
    pub fn result(&self) -> Result<GuestCallValue, RegionError> {
        if self.phase != Phase::Epilogue {
            return Err(RegionError::Incomplete);
        }
        self.output.clone().ok_or(RegionError::Incomplete)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, GuestMapOptions, GuestPermissions, ModuleIdentity};

    fn test_host() -> SyntheticRegionHost {
        let module = ModuleIdentity::new(
            ProviderId::new("test", "region"),
            "region.so",
            ContentDigest {
                algorithm: "none".to_string(),
                value: "0".to_string(),
            },
            "r1",
        );
        let mut memory = SparseGuestMemory::new(module, 4, 0x50000).unwrap();
        let image_base = memory
            .map(&GuestMapOptions::new(
                0x10000,
                0x1000,
                GuestPermissions::ReadWriteExecute,
            ))
            .unwrap();
        memory
            .map(&GuestMapOptions::new(0x80000, 0x1000, GuestPermissions::ReadWrite))
            .unwrap();
        SyntheticRegionHost::new(memory, image_base, 0x80800)
    }

    fn definition() -> ProtectionRegion {
        ProtectionRegion {
            frame: RegionFrame {
                entry: 0x100,
                exit: 0x400,
                stack_bytes: 16,
                argument_bytes: 8,
            },
            entry: 0x200,
            join: 0x300,
            inputs: vec![
                RegionInput {
                    target: RegionLocation::Stack {
                        offset: 20,
                        storage: GuestStorage::Int32,
                    },
                    value: GuestCallValue::Int32(41),
                },
                RegionInput {
                    target: RegionLocation::Register {
                        register: "eax".to_string(),
                        storage: GuestStorage::Uint32,
                    },
                    value: GuestCallValue::Uint32(7),
                },
            ],
            result: RegionLocation::Stack {
                offset: 24,
                storage: GuestStorage::Int32,
            },
            call_skips: 0,
        }
    }

    #[test]
    fn full_cycle_stages_inputs_captures_output_and_restores() {
        let mut host = test_host();
        host.set_stack_pointer(0x80800 - 16);
        let live = Rc::new(std::cell::Cell::new(true));
        let probe = live.clone();
        let mut execution = NativeModRegionExecution::new(
            &mut host,
            definition(),
            vec![GuestCallValue::Int32(41), GuestCallValue::Uint32(7)],
            Some(RegionAuthority::new(move || probe.get())),
        )
        .unwrap();
        host.set_stack_pointer(0x80800);
        execution.observe_entry(&host);
        host.set_stack_pointer(0x80800 - 16);
        execution.run_prologue(&mut host).unwrap();
        assert_eq!(host.read_register("eax"), 7);
        execution
            .run_region(&mut host, |host| {
                let address = host.stack_address(24);
                host.memory.write(address, &99i32.to_le_bytes()).unwrap();
                host.registers.insert("eax".to_string(), 0xDEAD);
            })
            .unwrap();
        execution.run_epilogue(&host).unwrap();
        assert_eq!(execution.result().unwrap(), GuestCallValue::Int32(99));
        assert_eq!(host.read_register("eax"), 0);
        let staged = host.stack_address(20);
        assert_eq!(host.memory.copy(staged, 4).unwrap(), vec![0, 0, 0, 0]);
        assert!(live.get());
    }

    #[test]
    fn validation_rejects_overlaps_and_stale_authority_aborts() {
        let mut overlapping = definition();
        overlapping.inputs.push(RegionInput {
            target: RegionLocation::Stack {
                offset: 22,
                storage: GuestStorage::Int32,
            },
            value: GuestCallValue::Int32(1),
        });
        assert_eq!(
            validate_native_mod_region(&overlapping, 4),
            Err(RegionError::OverlappingInputs)
        );
        let mut bad = definition();
        bad.join = bad.entry;
        assert!(matches!(
            validate_native_mod_region(&bad, 4),
            Err(RegionError::InvalidFrame(_))
        ));

        let mut host = test_host();
        let mut execution = NativeModRegionExecution::new(
            &mut host,
            definition(),
            vec![GuestCallValue::Int32(41), GuestCallValue::Uint32(7)],
            Some(RegionAuthority::new(|| false)),
        )
        .unwrap();
        execution.observe_entry(&host);
        assert_eq!(execution.run_prologue(&mut host), Err(RegionError::Retired));
    }
}
