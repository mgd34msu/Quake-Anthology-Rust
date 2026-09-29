//! Guest-address view over one QVM allocation for host callers.
//!
//! Port of `src/compat/qvm/guest-memory.ts` (`QvmGuestMemory`). Host addresses
//! refer to the same raw allocation read by QVM `LOAD`/`STORE`. Pointers are
//! 4 bytes; addresses carry the owning address-space token and reject foreign
//! spaces and overruns.

use crate::core::contracts::{fresh_address_space, GuestAddress, ModuleIdentity};
use crate::error::GuestError;

use super::memory::{QvmMemory, QvmWritableView};

/// Guest-address view over one module's QVM allocation.
#[derive(Debug, Clone)]
pub struct QvmGuestMemory {
    module: ModuleIdentity,
    memory: QvmMemory,
    space: u64,
}

impl QvmGuestMemory {
    /// Wrap `memory` for `module` in a fresh address space.
    #[must_use]
    pub fn new(module: ModuleIdentity, memory: QvmMemory) -> Self {
        Self {
            module,
            memory,
            space: fresh_address_space(),
        }
    }

    /// Owning module.
    #[must_use]
    pub fn module(&self) -> &ModuleIdentity {
        &self.module
    }

    /// Address-space token.
    #[must_use]
    pub fn address_space(&self) -> u64 {
        self.space
    }

    /// Pointer width in bytes (always 4).
    #[must_use]
    pub fn pointer_bytes(&self) -> usize {
        4
    }

    /// Mask a raw 32-bit value to an address. Zero maps to null; values
    /// outside the 32-bit representation fail.
    pub fn pointer(&self, raw: i64) -> Result<Option<GuestAddress>, GuestError> {
        if raw < -0x8000_0000 || raw > 0xffff_ffff {
            return Err(GuestError::invalid("QVM pointer is outside its 32-bit representation"));
        }
        let word = raw as u32 as i32;
        let Some(span) = self.memory.pointer(word)? else {
            return Ok(None);
        };
        self.address(span.start() as u64).map(Some)
    }

    fn address(&self, byte_offset: u64) -> Result<GuestAddress, GuestError> {
        if byte_offset > self.memory.len() as u64 {
            return Err(GuestError::invalid("QVM address exceeds allocation"));
        }
        Ok(GuestAddress::new(self.space, byte_offset))
    }

    fn checked(&self, address: GuestAddress, byte_length: usize) -> Result<usize, GuestError> {
        if address.space != self.space {
            return Err(GuestError::invalid("QVM pointer belongs to another module instance"));
        }
        if address.offset > self.memory.len() as u64 || byte_length as u64 > self.memory.len() as u64 - address.offset {
            return Err(GuestError::memory_fault(
                "out-of-bounds",
                address.offset,
                byte_length,
                "access",
                "QVM address range exceeds allocation",
            ));
        }
        Ok(address.offset as usize)
    }

    /// Offset an owned address by a signed displacement.
    pub fn offset(&self, address: GuestAddress, displacement: i64) -> Result<GuestAddress, GuestError> {
        self.checked(address, 0)?;
        let next = address.offset as i64 + displacement;
        if next < 0 {
            return Err(GuestError::invalid("QVM address exceeds allocation"));
        }
        self.address(next as u64)
    }

    /// Borrow a live scalar view over `byte_length` bytes at `address`.
    pub fn borrow(&self, address: GuestAddress, byte_length: usize) -> Result<QvmWritableView, GuestError> {
        let start = self.checked(address, byte_length)?;
        self.memory.data_view(start, byte_length)
    }

    /// Copy `byte_length` bytes at `address` out.
    pub fn copy(&self, address: GuestAddress, byte_length: usize) -> Result<Vec<u8>, GuestError> {
        let start = self.checked(address, byte_length)?;
        self.memory.read_bytes(start, byte_length)
    }

    /// Write `bytes` at `address`.
    pub fn write(&self, address: GuestAddress, bytes: &[u8]) -> Result<(), GuestError> {
        let start = self.checked(address, bytes.len())?;
        self.memory.write_bytes(start, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::contracts::ContentDigest;
    use qa_core::identity::ProviderId;

    fn guest() -> QvmGuestMemory {
        let module = ModuleIdentity::new(
            ProviderId::new("qvm", "test"),
            "vm/test.qvm",
            ContentDigest::new("sha256", "abc"),
            "1",
        );
        let memory = QvmMemory::new(vec![0; 64]).unwrap();
        memory.write_bytes(8, &[1, 2, 3, 4]).unwrap();
        QvmGuestMemory::new(module, memory)
    }

    #[test]
    fn pointers_mask_and_null_at_zero() {
        let guest = guest();
        assert_eq!(guest.pointer_bytes(), 4);
        assert!(guest.pointer(0).unwrap().is_none());
        let address = guest.pointer(8).unwrap().unwrap();
        assert_eq!(address.offset, 8);
        assert_eq!(address.space, guest.address_space());
        let wrapped = guest.pointer(-1).unwrap().unwrap();
        assert_eq!(wrapped.offset, 63);
        assert!(guest.pointer(0x1_0000_0000).is_err());
    }

    #[test]
    fn borrow_copy_write_share_the_allocation() {
        let guest = guest();
        let address = guest.pointer(8).unwrap().unwrap();
        assert_eq!(guest.copy(address, 4).unwrap(), vec![1, 2, 3, 4]);
        guest.write(address, &[9, 9]).unwrap();
        assert_eq!(guest.copy(address, 4).unwrap(), vec![9, 9, 3, 4]);
        let view = guest.borrow(address, 4).unwrap();
        view.set_u8(3, 7).unwrap();
        assert_eq!(guest.copy(address, 4).unwrap(), vec![9, 9, 3, 7]);
        let moved = guest.offset(address, 2).unwrap();
        assert_eq!(moved.offset, 10);
    }

    #[test]
    fn foreign_and_overrunning_addresses_fail() {
        let guest = guest();
        let foreign = GuestAddress::new(guest.address_space() + 1, 0);
        assert!(guest.copy(foreign, 1).is_err());
        let address = guest.pointer(63).unwrap().unwrap();
        assert!(guest.copy(address, 2).is_err());
        assert!(guest.offset(address, -64).is_err());
    }
}
