//! Guest VM core: execution contracts shared by memory, CPUs, ABI, and loaders.
//!
//! Donor: `src/guest/core/contracts.ts` plus the guest-owned subset of
//! `src/contracts/execution.ts` (`GuestAddress`, layouts, call values,
//! images, checkpoints). The donor's `symbol` address-space tokens become a
//! process-unique `u64` minted per memory; `bigint` offsets become `u64` with
//! explicit wrapping at the pointer width.

use std::sync::atomic::{AtomicU64, Ordering};

use qa_core::identity::ProviderId;

static NEXT_ADDRESS_SPACE: AtomicU64 = AtomicU64::new(1);

/// Mint a fresh guest address-space token.
#[must_use]
pub fn fresh_address_space() -> u64 {
    // Zero is reserved: it never names a live address space.
    let space = NEXT_ADDRESS_SPACE.fetch_add(1, Ordering::Relaxed);
    if space == 0 {
        NEXT_ADDRESS_SPACE.fetch_add(1, Ordering::Relaxed)
    } else {
        space
    }
}

/// Guest byte offset bound to the address space that owns it.
///
/// Pointers intentionally carry no actor generation; the owning memory
/// validates every address before use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GuestAddress {
    /// Owning address-space token.
    pub space: u64,
    /// Byte offset within the address space.
    pub offset: u64,
}

impl GuestAddress {
    /// Build an address in `space` at `offset`.
    #[must_use]
    pub const fn new(space: u64, offset: u64) -> Self {
        Self { space, offset }
    }
}

/// Stable callback name (`namespace:name`), separate from guest addresses.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CallbackId {
    /// Namespace portion.
    pub namespace: String,
    /// Name portion.
    pub name: String,
}

impl CallbackId {
    /// Build a callback name from its two parts.
    #[must_use]
    pub fn new(namespace: &str, name: &str) -> Self {
        Self {
            namespace: namespace.to_string(),
            name: name.to_string(),
        }
    }
}

impl std::fmt::Display for CallbackId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.namespace, self.name)
    }
}

/// Content digest identifying a module artifact.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContentDigest {
    /// Digest algorithm label.
    pub algorithm: String,
    /// Hex digest value.
    pub value: String,
}

impl ContentDigest {
    /// Build a digest from its parts.
    #[must_use]
    pub fn new(algorithm: &str, value: &str) -> Self {
        Self {
            algorithm: algorithm.to_string(),
            value: value.to_string(),
        }
    }
}

/// A replacement identifies the artifact it replaces, not just a module name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModuleIdentity {
    /// Stable implementation name.
    pub id: ProviderId,
    /// Artifact path the module was loaded from.
    pub artifact_path: String,
    /// Artifact digest.
    pub digest: ContentDigest,
    /// Source revision.
    pub revision: String,
}

impl ModuleIdentity {
    /// Build a module identity from its parts.
    #[must_use]
    pub fn new(id: ProviderId, artifact_path: &str, digest: ContentDigest, revision: &str) -> Self {
        Self {
            id,
            artifact_path: artifact_path.to_string(),
            digest,
            revision: revision.to_string(),
        }
    }
}

/// Native ABI of a loaded guest image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeAbi {
    /// 32-bit Windows PE with cdecl calls.
    WindowsI386,
    /// 64-bit Windows PE+ with the Microsoft x64 convention.
    WindowsX86_64,
    /// 32-bit Linux ELF with System V i386 calls.
    LinuxI386,
    /// 64-bit Linux ELF with System V AMD64 calls.
    LinuxX86_64,
}

impl NativeAbi {
    /// Pointer width in bytes.
    #[must_use]
    pub const fn pointer_bytes(self) -> usize {
        match self {
            Self::WindowsI386 | Self::LinuxI386 => 4,
            Self::WindowsX86_64 | Self::LinuxX86_64 => 8,
        }
    }

    /// Image format label.
    #[must_use]
    pub const fn image(self) -> &'static str {
        match self {
            Self::WindowsI386 => "pe32",
            Self::WindowsX86_64 => "pe32+",
            Self::LinuxI386 => "elf32",
            Self::LinuxX86_64 => "elf64",
        }
    }

    /// Default calling convention label.
    #[must_use]
    pub const fn call(self) -> &'static str {
        match self {
            Self::WindowsI386 => "cdecl",
            Self::WindowsX86_64 => "microsoft-x64",
            Self::LinuxI386 => "system-v-i386",
            Self::LinuxX86_64 => "system-v-x86-64",
        }
    }
}

/// Calling convention for one call. Win32 runtime imports and callbacks may
/// use a convention distinct from `GetGameAPI`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeCallAbi {
    /// 32-bit Windows cdecl.
    Cdecl,
    /// 32-bit Windows stdcall.
    Stdcall,
    /// 32-bit Windows thiscall.
    Thiscall,
    /// 32-bit Windows fastcall.
    Fastcall,
    /// 64-bit Windows Microsoft x64.
    MicrosoftX64,
    /// 32-bit Linux System V i386.
    SystemVI386,
    /// 64-bit Linux System V AMD64.
    SystemVX86_64,
}

impl NativeCallAbi {
    /// Pointer width in bytes.
    #[must_use]
    pub const fn pointer_bytes(self) -> usize {
        match self {
            Self::Cdecl | Self::Stdcall | Self::Thiscall | Self::Fastcall | Self::SystemVI386 => 4,
            Self::MicrosoftX64 | Self::SystemVX86_64 => 8,
        }
    }

    /// Convention label.
    #[must_use]
    pub const fn call(self) -> &'static str {
        match self {
            Self::Cdecl => "cdecl",
            Self::Stdcall => "stdcall",
            Self::Thiscall => "thiscall",
            Self::Fastcall => "fastcall",
            Self::MicrosoftX64 => "microsoft-x64",
            Self::SystemVI386 => "system-v-i386",
            Self::SystemVX86_64 => "system-v-x86-64",
        }
    }

    /// Parent image ABI kind label.
    #[must_use]
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Cdecl | Self::Stdcall | Self::Thiscall | Self::Fastcall => "windows-i386",
            Self::MicrosoftX64 => "windows-x86-64",
            Self::SystemVI386 => "linux-i386",
            Self::SystemVX86_64 => "linux-x86-64",
        }
    }

    /// Default call ABI for an image ABI.
    #[must_use]
    pub const fn of(abi: NativeAbi) -> Self {
        match abi {
            NativeAbi::WindowsI386 => Self::Cdecl,
            NativeAbi::WindowsX86_64 => Self::MicrosoftX64,
            NativeAbi::LinuxI386 => Self::SystemVI386,
            NativeAbi::LinuxX86_64 => Self::SystemVX86_64,
        }
    }
}

/// Scalar storage of one guest field or value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuestStorage {
    /// Signed 8-bit integer.
    Int8,
    /// Unsigned 8-bit integer.
    Uint8,
    /// Signed 16-bit integer.
    Int16,
    /// Unsigned 16-bit integer.
    Uint16,
    /// Signed 32-bit integer.
    Int32,
    /// Unsigned 32-bit integer.
    Uint32,
    /// Signed 64-bit integer.
    Int64,
    /// Unsigned 64-bit integer.
    Uint64,
    /// 32-bit float.
    Float32,
    /// 64-bit float.
    Float64,
    /// Guest pointer at the ambient width.
    Pointer,
}

impl GuestStorage {
    /// Storage width in bytes at `pointer_bytes`.
    #[must_use]
    pub const fn byte_length(self, pointer_bytes: usize) -> usize {
        match self {
            Self::Int8 | Self::Uint8 => 1,
            Self::Int16 | Self::Uint16 => 2,
            Self::Int32 | Self::Uint32 | Self::Float32 => 4,
            Self::Int64 | Self::Uint64 | Self::Float64 => 8,
            Self::Pointer => pointer_bytes,
        }
    }

    /// Whether the storage is a float lane.
    #[must_use]
    pub const fn is_float(self) -> bool {
        matches!(self, Self::Float32 | Self::Float64)
    }
}

/// Layout of one field inside a guest record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestFieldLayout {
    /// Field name.
    pub name: String,
    /// Byte offset within the record.
    pub byte_offset: usize,
    /// Element storage.
    pub storage: GuestStorage,
    /// Element count.
    pub count: usize,
}

/// Complete record layout: length, alignment, pointer width, fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestLayout {
    /// Layout identity (`namespace:name`).
    pub id: String,
    /// Record length in bytes.
    pub byte_length: usize,
    /// Record alignment in bytes.
    pub alignment: usize,
    /// Pointer width in bytes.
    pub pointer_bytes: usize,
    /// Ordered fields.
    pub fields: Vec<GuestFieldLayout>,
}

impl GuestLayout {
    /// Build a layout from its parts.
    #[must_use]
    pub fn new(
        id: &str,
        byte_length: usize,
        alignment: usize,
        pointer_bytes: usize,
        fields: Vec<GuestFieldLayout>,
    ) -> Self {
        Self {
            id: id.to_string(),
            byte_length,
            alignment,
            pointer_bytes,
            fields,
        }
    }
}

/// Value layout of one call parameter or result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestValueLayout {
    /// Scalar storage.
    Scalar(GuestStorage),
    /// Aggregate record.
    Aggregate(GuestLayout),
}

/// One marshalled call argument.
#[derive(Debug, Clone, PartialEq)]
pub enum GuestCallValue {
    /// Signed 32-bit integer.
    Int32(i32),
    /// Unsigned 32-bit integer.
    Uint32(u32),
    /// Signed 64-bit integer.
    Int64(i64),
    /// Unsigned 64-bit integer.
    Uint64(u64),
    /// 32-bit float.
    Float32(f32),
    /// 64-bit float.
    Float64(f64),
    /// Guest pointer (null decoded before exposure).
    Pointer(Option<GuestAddress>),
    /// Aggregate record bytes with their layout.
    Aggregate {
        /// Record layout.
        layout: GuestLayout,
        /// Little-endian record bytes.
        bytes: Vec<u8>,
    },
}

/// Result of one guest or host call.
#[derive(Debug, Clone, PartialEq)]
pub enum GuestCallResult {
    /// No value returned.
    Void,
    /// Returned value.
    Value(GuestCallValue),
}

/// Committed bytes relative to the observer's requested start, including
/// alias writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestWrittenRange {
    /// Offset from the observed start.
    pub byte_offset: usize,
    /// Committed length in bytes.
    pub byte_length: usize,
}

/// Guest pointer width in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuestPointerBytes {
    /// 32-bit pointers.
    Four = 4,
    /// 64-bit pointers.
    Eight = 8,
}

impl GuestPointerBytes {
    /// Width in bytes.
    #[must_use]
    pub const fn bytes(self) -> usize {
        self as usize
    }

    /// Convert a raw width, rejecting anything but 4 or 8.
    #[must_use]
    pub const fn of(bytes: usize) -> Option<Self> {
        match bytes {
            4 => Some(Self::Four),
            8 => Some(Self::Eight),
            _ => None,
        }
    }
}

/// Requested memory access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuestAccess {
    /// Load bytes.
    Read,
    /// Store bytes.
    Write,
    /// Fetch instructions.
    Execute,
}

impl GuestAccess {
    /// Access label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Execute => "execute",
        }
    }
}

/// Mapping permission set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuestPermissions {
    /// No access.
    None,
    /// Loads only.
    Read,
    /// Loads and stores.
    ReadWrite,
    /// Loads and instruction fetch.
    ReadExecute,
    /// Loads, stores, and instruction fetch.
    ReadWriteExecute,
    /// Instruction fetch only.
    Execute,
}

impl GuestPermissions {
    /// Whether the set grants `access`.
    #[must_use]
    pub const fn allows(self, access: GuestAccess) -> bool {
        match access {
            GuestAccess::Read => matches!(
                self,
                Self::Read | Self::ReadWrite | Self::ReadExecute | Self::ReadWriteExecute
            ),
            GuestAccess::Write => matches!(self, Self::ReadWrite | Self::ReadWriteExecute),
            GuestAccess::Execute => matches!(self, Self::Execute | Self::ReadExecute | Self::ReadWriteExecute),
        }
    }

    /// Permission label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Read => "read",
            Self::ReadWrite => "read-write",
            Self::ReadExecute => "read-execute",
            Self::ReadWriteExecute => "read-write-execute",
            Self::Execute => "execute",
        }
    }
}

/// One mapped guest range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestMapping {
    /// Mapping base offset.
    pub base: u64,
    /// Mapping length in bytes.
    pub byte_length: usize,
    /// Granted permissions.
    pub permissions: GuestPermissions,
    /// Diagnostic label.
    pub label: String,
}

/// Options for [`MappedGuestMemory::map`](super::memory::SparseGuestMemory::map).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestMapOptions {
    /// Mapping base offset.
    pub base: u64,
    /// Mapping length in bytes.
    pub byte_length: usize,
    /// Granted permissions.
    pub permissions: GuestPermissions,
    /// Diagnostic label.
    pub label: String,
    /// Initial bytes, copied into private storage; the remainder is zero-filled.
    pub bytes: Option<Vec<u8>>,
}

impl GuestMapOptions {
    /// Build map options from base, length, and permissions.
    #[must_use]
    pub fn new(base: u64, byte_length: usize, permissions: GuestPermissions) -> Self {
        Self {
            base,
            byte_length,
            permissions,
            label: String::new(),
            bytes: None,
        }
    }
}

/// Options for
/// [`MappedGuestMemory::allocate`](super::memory::SparseGuestMemory::allocate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestAllocationOptions {
    /// Allocation length in bytes.
    pub byte_length: usize,
    /// Base alignment; a positive power of two (default 16).
    pub alignment: u64,
    /// Granted permissions (default read-write).
    pub permissions: GuestPermissions,
    /// Diagnostic label.
    pub label: String,
}

impl GuestAllocationOptions {
    /// Build allocation options for `byte_length`.
    #[must_use]
    pub fn bytes(byte_length: usize) -> Self {
        Self {
            byte_length,
            alignment: 16,
            permissions: GuestPermissions::ReadWrite,
            label: "allocation".to_string(),
        }
    }
}

/// Snapshot of one mapping: identity plus its backing slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestSnapshotMapping {
    /// Mapping base offset.
    pub base: u64,
    /// Mapping length in bytes.
    pub byte_length: usize,
    /// Granted permissions.
    pub permissions: GuestPermissions,
    /// Diagnostic label.
    pub label: String,
    /// Backing index in the snapshot.
    pub backing: usize,
    /// Offset within the backing.
    pub backing_offset: usize,
}

/// Restorable memory image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestMemorySnapshot {
    /// Owning module.
    pub module: ModuleIdentity,
    /// Pointer width in bytes.
    pub pointer_bytes: usize,
    /// Allocation base.
    pub allocation_base: u64,
    /// Private backing images.
    pub backings: Vec<Vec<u8>>,
    /// Mappings over the backings.
    pub mappings: Vec<GuestSnapshotMapping>,
}

/// Guest integer register name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuestRegister {
    /// Accumulator.
    Rax,
    /// Counter.
    Rcx,
    /// Data.
    Rdx,
    /// Base.
    Rbx,
    /// Stack pointer.
    Rsp,
    /// Base pointer.
    Rbp,
    /// Source index.
    Rsi,
    /// Destination index.
    Rdi,
    /// Extended 8.
    R8,
    /// Extended 9.
    R9,
    /// Extended 10.
    R10,
    /// Extended 11.
    R11,
    /// Extended 12.
    R12,
    /// Extended 13.
    R13,
    /// Extended 14.
    R14,
    /// Extended 15.
    R15,
}

impl GuestRegister {
    /// Physical slot index.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Rax => 0,
            Self::Rcx => 1,
            Self::Rdx => 2,
            Self::Rbx => 3,
            Self::Rsp => 4,
            Self::Rbp => 5,
            Self::Rsi => 6,
            Self::Rdi => 7,
            Self::R8 => 8,
            Self::R9 => 9,
            Self::R10 => 10,
            Self::R11 => 11,
            Self::R12 => 12,
            Self::R13 => 13,
            Self::R14 => 14,
            Self::R15 => 15,
        }
    }

    /// Register name label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Rax => "rax",
            Self::Rcx => "rcx",
            Self::Rdx => "rdx",
            Self::Rbx => "rbx",
            Self::Rsp => "rsp",
            Self::Rbp => "rbp",
            Self::Rsi => "rsi",
            Self::Rdi => "rdi",
            Self::R8 => "r8",
            Self::R9 => "r9",
            Self::R10 => "r10",
            Self::R11 => "r11",
            Self::R12 => "r12",
            Self::R13 => "r13",
            Self::R14 => "r14",
            Self::R15 => "r15",
        }
    }

    /// Decode a 4-bit register index with a REX extension bit.
    #[must_use]
    pub const fn decode(code: u8, extended: bool) -> Self {
        match (code & 7) | ((extended as u8) << 3) {
            0 => Self::Rax,
            1 => Self::Rcx,
            2 => Self::Rdx,
            3 => Self::Rbx,
            4 => Self::Rsp,
            5 => Self::Rbp,
            6 => Self::Rsi,
            7 => Self::Rdi,
            8 => Self::R8,
            9 => Self::R9,
            10 => Self::R10,
            11 => Self::R11,
            12 => Self::R12,
            13 => Self::R13,
            14 => Self::R14,
            _ => Self::R15,
        }
    }
}

/// Integer operand width in bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuestIntegerWidth {
    /// 8-bit.
    B8 = 8,
    /// 16-bit.
    B16 = 16,
    /// 32-bit.
    B32 = 32,
    /// 64-bit.
    B64 = 64,
}

impl GuestIntegerWidth {
    /// Width in bits.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self as u32
    }

    /// Width in bytes.
    #[must_use]
    pub const fn bytes(self) -> usize {
        self as usize / 8
    }
}

/// Guest processor architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuestArchitecture {
    /// 32-bit i386.
    I386,
    /// 64-bit x86-64.
    X86_64,
}

impl GuestArchitecture {
    /// Addressable bits.
    #[must_use]
    pub const fn bits(self) -> u32 {
        match self {
            Self::I386 => 32,
            Self::X86_64 => 64,
        }
    }

    /// Pointer width in bytes.
    #[must_use]
    pub const fn pointer_bytes(self) -> usize {
        match self {
            Self::I386 => 4,
            Self::X86_64 => 8,
        }
    }
}

/// Processor status flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuestFlag {
    /// Carry.
    Carry,
    /// Parity.
    Parity,
    /// Auxiliary carry.
    AuxiliaryCarry,
    /// Zero.
    Zero,
    /// Sign.
    Sign,
    /// Trap.
    Trap,
    /// Interrupt enable.
    Interrupt,
    /// Direction.
    Direction,
    /// Overflow.
    Overflow,
    /// Resume.
    Resume,
    /// Virtual-8086 mode.
    Virtual8086,
    /// Alignment check.
    AlignmentCheck,
    /// Virtual interrupt.
    VirtualInterrupt,
    /// Virtual interrupt pending.
    VirtualInterruptPending,
    /// Identification.
    Identification,
}

impl GuestFlag {
    /// Status-word mask.
    #[must_use]
    pub const fn mask(self) -> u32 {
        match self {
            Self::Carry => 0x1,
            Self::Parity => 0x4,
            Self::AuxiliaryCarry => 0x10,
            Self::Zero => 0x40,
            Self::Sign => 0x80,
            Self::Trap => 0x100,
            Self::Interrupt => 0x200,
            Self::Direction => 0x400,
            Self::Overflow => 0x800,
            Self::Resume => 0x10000,
            Self::Virtual8086 => 0x20000,
            Self::AlignmentCheck => 0x40000,
            Self::VirtualInterrupt => 0x80000,
            Self::VirtualInterruptPending => 0x100000,
            Self::Identification => 0x200000,
        }
    }
}

/// Segment descriptor: selector, base, and limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestSegment {
    /// Segment selector.
    pub selector: u16,
    /// Segment base offset.
    pub base: u64,
    /// Segment limit.
    pub limit: u64,
}

impl GuestSegment {
    /// Flat segment covering `limit`.
    #[must_use]
    pub const fn flat(limit: u64) -> Self {
        Self {
            selector: 0,
            base: 0,
            limit,
        }
    }
}

/// Decoded faulting instruction for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestInstruction {
    /// Instruction address.
    pub address: GuestAddress,
    /// Raw instruction bytes.
    pub bytes: Vec<u8>,
    /// Mnemonic.
    pub mnemonic: String,
}

/// Guest execution fault.
#[derive(Debug, Clone, PartialEq)]
pub enum GuestException {
    /// Processor exception with vector and optional error code.
    Processor {
        /// Exception vector.
        vector: u32,
        /// Error code, if pushed.
        error_code: Option<u64>,
        /// Faulting instruction address.
        instruction: GuestAddress,
        /// Human-readable detail.
        detail: String,
    },
    /// Memory access fault.
    Memory {
        /// Requested access.
        access: GuestAccess,
        /// Faulting address.
        address: GuestAddress,
        /// Requested length.
        byte_length: usize,
        /// Human-readable detail.
        detail: String,
    },
    /// Windows structured exception.
    Windows {
        /// Exception code.
        code: u32,
        /// Exception flags.
        flags: u32,
        /// Faulting address.
        address: GuestAddress,
        /// Exception arguments.
        arguments: Vec<u64>,
    },
    /// C++ exception object in flight.
    Cxx {
        /// Exception object address.
        object: GuestAddress,
        /// Type info address, if known.
        type_info: Option<GuestAddress>,
        /// Destructor address, if known.
        destructor: Option<GuestAddress>,
        /// Throwing ABI.
        abi: NativeAbi,
    },
}

/// Reason a [`GuestCpu`](super::cpu::GuestCpu) run stopped.
#[derive(Debug, Clone, PartialEq)]
pub enum GuestExecutionStop {
    /// Instruction budget exhausted.
    Budget {
        /// Retired instructions.
        instructions: u64,
    },
    /// Returned through the return address.
    Return {
        /// Retired instructions.
        instructions: u64,
        /// Return address.
        address: GuestAddress,
    },
    /// Reached a host-call trap.
    HostCall {
        /// Retired instructions.
        instructions: u64,
        /// Trap address.
        address: GuestAddress,
    },
    /// Executed a halt.
    Halt {
        /// Retired instructions.
        instructions: u64,
        /// Halt address.
        address: GuestAddress,
    },
    /// Raised an exception.
    Exception {
        /// Retired instructions.
        instructions: u64,
        /// Raised exception.
        exception: GuestException,
    },
    /// Hit an undecodable or unimplemented instruction.
    Unsupported {
        /// Retired instructions.
        instructions: u64,
        /// Offending instruction.
        instruction: GuestInstruction,
        /// Human-readable detail.
        detail: String,
    },
}

impl GuestExecutionStop {
    /// Stop-kind label.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Budget { .. } => "budget",
            Self::Return { .. } => "return",
            Self::HostCall { .. } => "host-call",
            Self::Halt { .. } => "halt",
            Self::Exception { .. } => "exception",
            Self::Unsupported { .. } => "unsupported",
        }
    }

    /// Retired instruction count.
    #[must_use]
    pub const fn instructions(&self) -> u64 {
        match self {
            Self::Budget { instructions }
            | Self::Return { instructions, .. }
            | Self::HostCall { instructions, .. }
            | Self::Halt { instructions, .. }
            | Self::Exception { instructions, .. }
            | Self::Unsupported { instructions, .. } => *instructions,
        }
    }
}

/// Signature of one guest/host call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestCallSignature {
    /// Calling convention.
    pub abi: NativeCallAbi,
    /// Parameter layouts.
    pub parameters: Vec<GuestValueLayout>,
    /// Result layout, or `None` for void.
    pub result: Option<GuestValueLayout>,
    /// C variadic tail.
    pub variadic: bool,
}

impl GuestCallSignature {
    /// Whether two signatures marshal identically.
    #[must_use]
    pub fn same_as(&self, other: &Self) -> bool {
        self.abi == other.abi
            && self.variadic == other.variadic
            && self.parameters == other.parameters
            && self.result == other.result
    }
}

/// Imported symbol reference: name or ordinal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestSymbolName {
    /// Named import with an optional version.
    Name {
        /// Symbol name.
        name: String,
        /// Symbol version, if any.
        version: Option<String>,
    },
    /// Ordinal import.
    Ordinal(u32),
}

/// One image import slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestImport {
    /// Providing library name.
    pub library: String,
    /// Imported symbol.
    pub symbol: GuestSymbolName,
    /// Import address-table slot.
    pub slot: GuestAddress,
    /// Weak import: null when unresolved.
    pub weak: bool,
}

/// Resolution of one import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestImportResolution {
    /// Defined by another guest image.
    Guest {
        /// Definition address.
        address: GuestAddress,
        /// Defining module.
        module: ModuleIdentity,
    },
    /// Served by a host callback trap.
    Host {
        /// Trap address.
        address: GuestAddress,
        /// Callback identity.
        callback: CallbackId,
    },
    /// Unresolved import with its detail.
    Unresolved {
        /// Unresolved import.
        import: GuestImport,
        /// Human-readable detail.
        detail: String,
    },
}

/// One image export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestExport {
    /// Exported symbol.
    pub symbol: GuestSymbolName,
    /// Export target.
    pub target: GuestExportTarget,
}

/// Export target: address or forwarder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestExportTarget {
    /// Direct definition address.
    Address(GuestAddress),
    /// Forwarded to another library symbol.
    Forward {
        /// Forward library name.
        library: String,
        /// Forward symbol.
        symbol: GuestSymbolName,
    },
}

/// Resolves image imports to guest definitions or host traps.
pub trait GuestImportResolver {
    /// Resolve one import requested by `requesting`. Runtimes may bind host
    /// traps or reserve guest pages through `memory` while resolving.
    fn resolve(
        &self,
        memory: &mut crate::core::memory::SparseGuestMemory,
        import: &GuestImport,
        requesting: &GuestImage,
    ) -> GuestImportResolution;
}

/// Thread-local-storage template of one image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestTlsTemplate {
    /// Owning image.
    pub image: ModuleIdentity,
    /// Initialized template bytes.
    pub initialized: Vec<u8>,
    /// Trailing zero-fill bytes.
    pub zero_fill_bytes: usize,
    /// TLS alignment.
    pub alignment: u64,
    /// TLS callbacks.
    pub callbacks: Vec<GuestAddress>,
}

/// Per-thread TLS block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestThreadTls {
    /// OS thread id.
    pub thread_id: u64,
    /// Thread pointer value.
    pub thread_pointer: GuestAddress,
    /// Per-module TLS base addresses.
    pub modules: Vec<(ModuleIdentity, GuestAddress)>,
}

/// Unwind metadata region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestUnwindRegion {
    /// Region start.
    pub start: GuestAddress,
    /// Region end.
    pub end: GuestAddress,
    /// Unwind format label.
    pub format: GuestUnwindFormat,
    /// Raw metadata bytes.
    pub metadata: Vec<u8>,
}

/// Unwind metadata format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuestUnwindFormat {
    /// Windows x64 `.pdata`/`.xdata`.
    PeX64Unwind,
    /// ELF `.eh_frame`.
    ElfEhFrame,
    /// ELF `.debug_frame`.
    ElfDebugFrame,
}

/// Reference to one callable guest or host entry point.
#[derive(Debug, Clone, PartialEq)]
pub enum GuestCallbackReference {
    /// TypeScript provider callback.
    TypeScript {
        /// Owning provider.
        provider: ProviderId,
        /// Callback identity.
        callback: CallbackId,
    },
    /// QuakeC function index.
    QuakeC {
        /// Owning module.
        module: ModuleIdentity,
        /// Function index.
        function_index: u32,
    },
    /// QVM instruction index.
    Qvm {
        /// Owning module.
        module: ModuleIdentity,
        /// Instruction index.
        instruction_index: u32,
    },
    /// Native guest address with its call ABI.
    NativeGuest {
        /// Owning module.
        module: ModuleIdentity,
        /// Entry address.
        address: GuestAddress,
        /// Call ABI.
        abi: NativeCallAbi,
    },
}

/// Saved native offsets are rebound into the restored address space.
#[derive(Debug, Clone, PartialEq)]
pub enum SavedGuestCallbackReference {
    /// TypeScript provider callback.
    TypeScript {
        /// Owning provider.
        provider: ProviderId,
        /// Callback identity.
        callback: CallbackId,
    },
    /// QuakeC function index.
    QuakeC {
        /// Owning module.
        module: ModuleIdentity,
        /// Function index.
        function_index: u32,
    },
    /// QVM instruction index.
    Qvm {
        /// Owning module.
        module: ModuleIdentity,
        /// Instruction index.
        instruction_index: u32,
    },
    /// Native guest offset rebound on restore.
    NativeGuest {
        /// Owning module.
        module: ModuleIdentity,
        /// Entry offset.
        byte_offset: u64,
        /// Call ABI.
        abi: NativeCallAbi,
    },
}

/// Raw view over one source-owned entity record. This view is never a
/// network snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct RawEntityView {
    /// Owning module.
    pub module: ModuleIdentity,
    /// Entity slot.
    pub slot: u32,
    /// Record address.
    pub address: GuestAddress,
    /// Record stride in bytes.
    pub stride_bytes: usize,
    /// Public record layout.
    pub public_layout: GuestLayout,
    /// Complete source-owned record bytes.
    pub bytes: Vec<u8>,
}

/// Context of one guest or host call.
#[derive(Debug, Clone, PartialEq)]
pub struct GuestCallContext {
    /// Calling module.
    pub module: ModuleIdentity,
    /// Called reference.
    pub callback: GuestCallbackReference,
    /// Enclosing call, if nested.
    pub parent: Option<Box<GuestCallContext>>,
    /// Calling entity, if any.
    pub itself: Option<RawEntityView>,
    /// Other entity, if any.
    pub other: Option<RawEntityView>,
}

/// Static binding of one callback.
#[derive(Debug, Clone, PartialEq)]
pub struct GuestCallbackBinding {
    /// Callback identity.
    pub id: CallbackId,
    /// Callable reference.
    pub reference: GuestCallbackReference,
    /// Parameter layouts.
    pub parameters: Vec<GuestValueLayout>,
    /// Result layout, or `None` for void.
    pub result: Option<GuestValueLayout>,
}

/// Saved binding with rebound native references.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedGuestCallbackBinding {
    /// Callback identity.
    pub id: CallbackId,
    /// Saved reference.
    pub reference: SavedGuestCallbackReference,
    /// Parameter layouts.
    pub parameters: Vec<GuestValueLayout>,
    /// Result layout, or `None` for void.
    pub result: Option<GuestValueLayout>,
}

/// Loaded executable image: mappings, imports, exports, TLS, unwind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestImage {
    /// Owning module.
    pub module: ModuleIdentity,
    /// Image ABI.
    pub abi: NativeAbi,
    /// Loaded base address.
    pub base: GuestAddress,
    /// Preferred base offset.
    pub preferred_base: u64,
    /// Image length in bytes.
    pub byte_length: u64,
    /// Entry-point address, if any.
    pub entry_point: Option<GuestAddress>,
    /// Image mappings.
    pub mappings: Vec<GuestMapping>,
    /// Image imports.
    pub imports: Vec<GuestImport>,
    /// Image exports.
    pub exports: Vec<GuestExport>,
    /// TLS template, if any.
    pub tls: Option<GuestTlsTemplate>,
    /// Initializer addresses in run order.
    pub initializers: Vec<GuestAddress>,
    /// Finalizer addresses in run order.
    pub finalizers: Vec<GuestAddress>,
    /// Unwind regions.
    pub unwind: Vec<GuestUnwindRegion>,
}
