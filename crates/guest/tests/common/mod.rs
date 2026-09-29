//! Shared fixtures for guest integration tests: synthetic code, stack,
//! and return-address setup mirroring the donor `tests/guest` harnesses.

use qa_core::identity::ProviderId;
use qa_guest::abi::GuestCpu;
use qa_guest::core::contracts::{
    ContentDigest, GuestAddress, GuestArchitecture, GuestExecutionStop, GuestMapOptions,
    GuestPermissions, ModuleIdentity,
};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::core::registers::{
    GuestProcessorInitialState, GuestProcessorState,
};
use qa_guest::x64::cpu::X64Cpu;

/// Fixture code base.
pub const BASE: u64 = 0x10000;
/// Fixture stack pointer.
pub const STACK: u64 = 0x30000;
/// Fixture return address.
pub const RETURNED: u64 = 0x40000;

/// Test module identity.
pub fn test_module(name: &str) -> ModuleIdentity {
    ModuleIdentity::new(
        ProviderId::new("test", name),
        "authored-test-bytes",
        ContentDigest::new("sha256", "abc"),
        "1",
    )
}

/// Resolve a mapped offset, panicking when unmapped.
pub fn addr(memory: &SparseGuestMemory, offset: u64) -> GuestAddress {
    memory
        .pointer(offset)
        .expect("fixture pointer")
        .expect("nonnull fixture address")
}

/// Map a fixture region.
pub fn map(
    memory: &mut SparseGuestMemory,
    base: u64,
    byte_length: usize,
    permissions: GuestPermissions,
    bytes: Option<Vec<u8>>,
) -> GuestAddress {
    memory
        .map(&GuestMapOptions {
            base,
            byte_length,
            permissions,
            label: "fixture".to_string(),
            bytes,
        })
        .expect("fixture map")
}

/// Fresh processor state and memory with `bytes` mapped executable at
/// `start`, a stack at [`STACK`], and [`RETURNED`] pushed as the return
/// address.
pub fn build(bytes: &[u8], start: u64, module: ModuleIdentity) -> (GuestProcessorState, SparseGuestMemory) {
    let mut memory = SparseGuestMemory::new(module, 8, 0x10000).expect("fixture memory");
    map(
        &mut memory,
        start,
        bytes.len(),
        GuestPermissions::ReadExecute,
        Some(bytes.to_vec()),
    );
    map(
        &mut memory,
        STACK - 0x1000,
        0x1008,
        GuestPermissions::ReadWrite,
        None,
    );
    let stack = addr(&memory, STACK);
    memory.write_u64(stack, RETURNED).expect("fixture return");
    let state = GuestProcessorState::create(GuestProcessorInitialState {
        architecture: GuestArchitecture::X86_64,
        instruction_pointer: start,
        stack_pointer: STACK,
        flags: 2,
        x87_control_word: 0x37f,
        mxcsr: 0x1f80,
        mxcsr_mask: 0xffff,
    })
    .expect("fixture state");
    (state, memory)
}

/// x86-64 fixture CPU.
pub struct Fixture {
    /// Fixture CPU (owns state and memory).
    pub cpu: X64Cpu,
    /// Fixture return address.
    pub returned: GuestAddress,
}

impl Fixture {
    /// Build a fixture at [`BASE`].
    pub fn new(bytes: &[u8]) -> Self {
        Self::at(bytes, BASE)
    }

    /// Build a fixture at `start`.
    pub fn at(bytes: &[u8], start: u64) -> Self {
        let (state, mut memory) = build(bytes, start, test_module("x64"));
        memory
            .map(&GuestMapOptions {
                base: RETURNED,
                byte_length: 8,
                permissions: GuestPermissions::ReadExecute,
                label: "fixture return".to_string(),
                bytes: None,
            })
            .expect("fixture return map");
        let returned = addr(&memory, RETURNED);
        let cpu = X64Cpu::new(state, memory).expect("fixture cpu");
        Self { cpu, returned }
    }

    /// Run with `budget`, stopping at the fixture return address.
    pub fn run(&mut self, budget: u64) -> GuestExecutionStop {
        self.cpu.run(budget, Some(self.returned))
    }

    /// Borrow state and memory together.
    pub fn with<R>(
        &mut self,
        f: impl FnOnce(&mut GuestProcessorState, &mut SparseGuestMemory) -> R,
    ) -> R {
        let (state, memory) = self.cpu.parts();
        f(state, memory)
    }

    /// Reset the instruction and stack pointers for another pass.
    pub fn reset(&mut self, start: u64) {
        use qa_guest::core::contracts::{GuestIntegerWidth, GuestRegister};
        self.with(|state, _| {
            state.instruction_pointer = start;
            state
                .registers
                .write(GuestRegister::Rsp, GuestIntegerWidth::B64, STACK, false)
                .unwrap();
        });
    }
}

fn pe_w16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn pe_w32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn pe_w64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn pe_wtext(bytes: &mut [u8], offset: usize, value: &str) {
    bytes[offset..offset + value.len()].copy_from_slice(value.as_bytes());
    bytes[offset + value.len()] = 0;
}

fn pe_raw_of(rva: usize) -> usize {
    if rva < 0x2000 {
        rva - 0x1000 + 0x400
    } else if rva < 0x3000 {
        rva - 0x2000 + 0x600
    } else if rva < 0x4000 {
        rva - 0x3000 + 0xe00
    } else {
        rva - 0x4000 + 0x1000
    }
}

pub fn pe_fixture(width: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; 0x1200];
    let base: u64 = if width == 4 { 0x1000_0000 } else { 0x1_8000_0000 };
    let optional = 0x98usize;
    let optional_size = if width == 4 { 224usize } else { 240usize };
    let directories = optional + if width == 4 { 96 } else { 112 };
    let dword = |bytes: &mut Vec<u8>, rva: usize, value: u32| pe_w32(bytes, pe_raw_of(rva), value);
    let pointer = |bytes: &mut Vec<u8>, rva: usize, value: u64| {
        if width == 4 {
            #[allow(clippy::cast_possible_truncation)]
            pe_w32(bytes, pe_raw_of(rva), value as u32);
        } else {
            pe_w64(bytes, pe_raw_of(rva), value);
        }
    };
    let directory = |bytes: &mut Vec<u8>, index: usize, rva: u32, size: u32| {
        pe_w32(bytes, directories + index * 8, rva);
        pe_w32(bytes, directories + index * 8 + 4, size);
    };
    pe_w16(&mut bytes, 0, 0x5a4d);
    pe_w32(&mut bytes, 0x3c, 0x80);
    pe_w32(&mut bytes, 0x80, 0x4550);
    pe_w16(&mut bytes, 0x84, if width == 4 { 0x14c } else { 0x8664 });
    pe_w16(&mut bytes, 0x86, 4);
    pe_w16(&mut bytes, 0x94, optional_size as u16);
    pe_w16(&mut bytes, 0x96, 0x2002);
    pe_w16(&mut bytes, optional, if width == 4 { 0x10b } else { 0x20b });
    pe_w32(&mut bytes, optional + 16, 0x1000);
    if width == 4 {
        pe_w32(&mut bytes, optional + 28, base as u32);
    } else {
        pe_w64(&mut bytes, optional + 24, base);
    }
    pe_w32(&mut bytes, optional + 32, 0x1000);
    pe_w32(&mut bytes, optional + 36, 0x200);
    pe_w32(&mut bytes, optional + 56, 0x5000);
    pe_w32(&mut bytes, optional + 60, 0x400);
    pe_w32(&mut bytes, directories - 4, 16);
    let sections = [
        (".text", 0x1000u32, 0x80u32, 0x400u32, 0x200u32, 0x6000_0020u32),
        (".rdata", 0x2000, 0x800, 0x600, 0x800, 0x4000_0040),
        (".data", 0x3000, 0x1000, 0xe00, 0x200, 0xc000_0040),
        (".reloc", 0x4000, 0x200, 0x1000, 0x200, 0x4200_0040),
    ];
    for (index, (name, rva, size, raw_offset, raw_size, flags)) in sections.iter().enumerate() {
        let at = optional + optional_size + index * 40;
        pe_wtext(&mut bytes, at, name);
        pe_w32(&mut bytes, at + 8, *size);
        pe_w32(&mut bytes, at + 12, *rva);
        pe_w32(&mut bytes, at + 16, *raw_size);
        pe_w32(&mut bytes, at + 20, *raw_offset);
        pe_w32(&mut bytes, at + 36, *flags);
    }
    bytes[0x400..0x480].fill(0xc3);
    directory(&mut bytes, 0, 0x2000, 0x100);
    dword(&mut bytes, 0x200c, 0x2090);
    dword(&mut bytes, 0x2010, 8);
    dword(&mut bytes, 0x2014, 2);
    dword(&mut bytes, 0x2018, 2);
    dword(&mut bytes, 0x201c, 0x2040);
    dword(&mut bytes, 0x2020, 0x2050);
    dword(&mut bytes, 0x2024, 0x2058);
    dword(&mut bytes, 0x2040, 0x1010);
    dword(&mut bytes, 0x2044, 0x2080);
    dword(&mut bytes, 0x2050, 0x2060);
    dword(&mut bytes, 0x2054, 0x2070);
    let r = pe_raw_of(0x2058);
    pe_w16(&mut bytes, r, 0);
    let r = pe_raw_of(0x205a);
    pe_w16(&mut bytes, r, 1);
    let r = pe_raw_of(0x2060);
    pe_wtext(&mut bytes, r, "GetGameAPI");
    let r = pe_raw_of(0x2070);
    pe_wtext(&mut bytes, r, "Forward");
    let r = pe_raw_of(0x2080);
    pe_wtext(&mut bytes, r, "other.#8");
    let r = pe_raw_of(0x2090);
    pe_wtext(&mut bytes, r, "authored.dll");
    directory(&mut bytes, 1, 0x2100, 40);
    dword(&mut bytes, 0x2100, 0x2140);
    dword(&mut bytes, 0x210c, 0x2180);
    dword(&mut bytes, 0x2110, 0x2160);
    pointer(&mut bytes, 0x2140, 0x21a0);
    pointer(&mut bytes, 0x2140 + width, (1u64 << (width * 8 - 1)) | 7);
    pointer(&mut bytes, 0x2160, 0x21a0);
    pointer(&mut bytes, 0x2160 + width, (1u64 << (width * 8 - 1)) | 7);
    let r = pe_raw_of(0x2180);
    pe_wtext(&mut bytes, r, "guest.dll");
    let r = pe_raw_of(0x21a2);
    pe_wtext(&mut bytes, r, "Target");
    directory(&mut bytes, 9, 0x2200, (width * 4 + 8) as u32);
    pointer(&mut bytes, 0x2200, base + 0x3000);
    pointer(&mut bytes, 0x2200 + width, base + 0x3004);
    pointer(&mut bytes, 0x2200 + width * 2, base + 0x3020);
    pointer(&mut bytes, 0x2200 + width * 3, base + 0x2240);
    dword(&mut bytes, 0x2200 + width * 4, 12);
    dword(&mut bytes, 0x2200 + width * 4 + 4, 0x0030_0000);
    pointer(&mut bytes, 0x2240, base + 0x1020);
    pointer(&mut bytes, 0x2240 + width, base + 0x1030);
    let r = pe_raw_of(0x3000);
    bytes[r..r + 4].copy_from_slice(&[9, 8, 7, 6]);
    pointer(&mut bytes, 0x3010, base + 0x1050);
    let config_size = if width == 4 { 92u32 } else { 148u32 };
    directory(&mut bytes, 10, 0x2280, config_size);
    dword(&mut bytes, 0x2280, config_size);
    let cookie = 0x2280 + if width == 4 { 60 } else { 88 };
    let check = 0x2280 + if width == 4 { 72 } else { 112 };
    let dispatch = 0x2280 + if width == 4 { 76 } else { 120 };
    pointer(&mut bytes, cookie, base + 0x3040);
    pointer(&mut bytes, check, base + 0x3060);
    pointer(&mut bytes, dispatch, base + 0x3070);
    dword(&mut bytes, 0x2280 + if width == 4 { 88 } else { 144 }, 0x100);
    if width == 8 {
        directory(&mut bytes, 3, 0x2380, 12);
        dword(&mut bytes, 0x2380, 0x1000);
        dword(&mut bytes, 0x2384, 0x1040);
        dword(&mut bytes, 0x2388, 0x23a0);
        let r = pe_raw_of(0x23a0);
        bytes[r..r + 8].copy_from_slice(&[1, 4, 1, 0, 4, 0x32, 0, 0]);
    }
    let relocation_type = if width == 4 { 3u16 } else { 10u16 };
    let mut rdata: Vec<u16> = [0x2200, 0x2200 + width, 0x2200 + width * 2, 0x2200 + width * 3, 0x2240, 0x2240 + width, cookie, check, dispatch]
        .iter()
        .map(|rva| (relocation_type << 12) | ((rva - 0x2000) as u16))
        .collect();
    rdata.push(0);
    let mut data = vec![(relocation_type << 12) | 0x10];
    if width == 4 {
        let r = pe_raw_of(0x3018);
        pe_w16(&mut bytes, r, 0x1122);
        let r = pe_raw_of(0x301a);
        pe_w16(&mut bytes, r, 0x3344);
        let r = pe_raw_of(0x301c);
        pe_w16(&mut bytes, r, 0x2000);
        data.extend([0x1018, 0x201a, 0x401c, 0x8123]);
    }
    data.push(0);
    let mut relocation_bytes = 0usize;
    for (page, entries) in [(0x2000u32, rdata), (0x3000u32, data)] {
        let size = 8 + entries.len() * 2;
        dword(&mut bytes, 0x4000 + relocation_bytes, page);
        dword(&mut bytes, 0x4004 + relocation_bytes, size as u32);
        for (index, value) in entries.iter().enumerate() {
            let r = pe_raw_of(0x4008 + relocation_bytes + index * 2);
            pe_w16(&mut bytes, r, *value);
        }
        relocation_bytes += size;
    }
    directory(&mut bytes, 5, 0x4000, relocation_bytes as u32);
    bytes
}

