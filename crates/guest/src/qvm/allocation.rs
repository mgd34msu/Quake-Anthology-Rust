//! QVM storage accounting: engine arenas can own VM bytes without owning execution.
//!
//! Port of `src/compat/qvm/allocation.ts` (`QvmAllocation`, `QvmAllocationProfile`).
//! An allocation owns its bytes plus a shared liveness flag; the arena that
//! issued it releases the flag, after which every read fails. Failures use
//! [`GuestError`](crate::error::GuestError).

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::GuestError;

/// Request passed to an accounting arena for one VM storage block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmAllocationRequest {
    /// Why the VM needs the bytes (for example `"VM_Create:dataBase"`).
    pub purpose: String,
    /// Resource the bytes belong to (usually the module artifact path).
    pub resource: String,
    /// Requested length in bytes.
    pub byte_length: usize,
}

/// One VM-owned storage block. Reading bytes must reject a released allocation.
#[derive(Debug, Clone)]
pub struct QvmAllocation {
    bytes: Vec<u8>,
    live: Rc<RefCell<bool>>,
}

impl QvmAllocation {
    /// Fresh live allocation with zeroed bytes.
    #[must_use]
    pub fn new(byte_length: usize) -> Self {
        Self {
            bytes: vec![0; byte_length],
            live: Rc::new(RefCell::new(true)),
        }
    }

    /// Whether the issuing arena still retains this allocation.
    #[must_use]
    pub fn is_live(&self) -> bool {
        *self.live.borrow()
    }

    /// Release the allocation; later reads fail.
    pub fn release(&self) {
        *self.live.borrow_mut() = false;
    }

    /// Byte length of the allocation.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether the allocation holds no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Borrow the bytes, rejecting a released allocation.
    pub fn bytes(&self) -> Result<&[u8], GuestError> {
        if !self.is_live() {
            return Err(GuestError::invalid("QVM allocation has been released"));
        }
        Ok(&self.bytes)
    }

    /// Mutably borrow the bytes, rejecting a released allocation.
    pub fn bytes_mut(&mut self) -> Result<&mut [u8], GuestError> {
        if !self.is_live() {
            return Err(GuestError::invalid("QVM allocation has been released"));
        }
        Ok(&mut self.bytes)
    }

    /// Take the bytes out of a live allocation, leaving it empty but live.
    pub fn take_bytes(&mut self) -> Result<Vec<u8>, GuestError> {
        self.bytes_mut()?;
        Ok(std::mem::take(&mut self.bytes))
    }
}

/// How a VM instance accounts its storage blocks.
pub enum QvmAllocationProfile {
    /// The VM owns plain zeroed bytes with no arena.
    Unaccounted,
    /// The arena issues every block and can release it later.
    Accounted(Box<dyn FnMut(&QvmAllocationRequest) -> Result<QvmAllocation, GuestError>>),
}

impl std::fmt::Debug for QvmAllocationProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unaccounted => write!(f, "Unaccounted"),
            Self::Accounted(_) => write!(f, "Accounted(..)"),
        }
    }
}

impl Default for QvmAllocationProfile {
    fn default() -> Self {
        Self::Unaccounted
    }
}

impl QvmAllocationProfile {
    /// Issue one storage block for `purpose`/`resource` with `byte_length` bytes.
    pub fn allocate(
        &mut self,
        purpose: &str,
        resource: &str,
        byte_length: usize,
    ) -> Result<QvmAllocation, GuestError> {
        match self {
            Self::Unaccounted => Ok(QvmAllocation::new(byte_length)),
            Self::Accounted(allocate) => allocate(&QvmAllocationRequest {
                purpose: purpose.to_string(),
                resource: resource.to_string(),
                byte_length,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unaccounted_profile_issues_zeroed_bytes() {
        let mut profile = QvmAllocationProfile::default();
        let allocation = profile.allocate("VM_Create:dataBase", "qagame", 16).unwrap();
        assert_eq!(allocation.bytes().unwrap(), &[0; 16]);
        assert!(allocation.is_live());
    }

    #[test]
    fn released_allocation_rejects_reads() {
        let allocation = QvmAllocation::new(8);
        allocation.release();
        assert!(!allocation.is_live());
        assert!(allocation.bytes().is_err());
    }

    #[test]
    fn accounted_profile_receives_request_details() {
        let mut profile = QvmAllocationProfile::Accounted(Box::new(|request| {
            assert_eq!(request.purpose, "VM_PrepareInterpreter");
            assert_eq!(request.resource, "cgame");
            assert_eq!(request.byte_length, 4);
            Ok(QvmAllocation::new(request.byte_length))
        }));
        let allocation = profile.allocate("VM_PrepareInterpreter", "cgame", 4).unwrap();
        assert_eq!(allocation.len(), 4);
    }

    #[test]
    fn release_is_shared_with_clones() {
        let allocation = QvmAllocation::new(4);
        let shared = allocation.clone();
        shared.release();
        assert!(allocation.bytes().is_err());
    }
}
