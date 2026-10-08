//! Fixed-capacity scratch storage. Allocate backing memory at load, reset per frame.
use std::{
    alloc::{Layout, alloc, dealloc},
    marker::PhantomData,
    ptr::NonNull,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_ARENA: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArenaError {
    InvalidCapacity,
    AllocationFailed,
    OutOfSpace,
    ZeroSizedType,
    GenerationExhausted,
}

#[derive(Clone, Copy, Debug)]
/// A typed range retains its exact element type across copied handles.
///
/// ```compile_fail
/// use qa_core::arena::{Block, FrameArena};
/// let mut arena = FrameArena::load(64).unwrap();
/// let block: Block<&'static str> = arena.allocate(1, "permanent").unwrap();
/// {
///     let short = String::from("temporary");
///     arena.get_mut(block).unwrap()[0] = short.as_str();
/// }
/// let retained: &'static str = arena.get(block).unwrap()[0];
/// ```
pub struct Block<T: Copy> {
    arena: u64,
    generation: u64,
    offset: usize,
    len: usize,
    // Invariant T: a copied handle must not permit replacing long-lived
    // references through a shortened-lifetime view of the same storage.
    element: PhantomData<(T, fn(T))>,
}

pub struct FrameArena {
    storage: NonNull<u8>,
    layout: Layout,
    capacity: usize,
    used: usize,
    id: u64,
    generation: u64,
}

impl FrameArena {
    pub fn load(capacity: usize) -> Result<Self, ArenaError> {
        let layout = Layout::from_size_align(capacity.max(1), 64)
            .map_err(|_| ArenaError::InvalidCapacity)?;
        let id = NEXT_ARENA
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| ArenaError::GenerationExhausted)?;
        // SAFETY: layout is nonempty and valid; this arena owns the allocation.
        let storage = NonNull::new(unsafe { alloc(layout) }).ok_or(ArenaError::AllocationFailed)?;
        Ok(Self {
            storage,
            layout,
            capacity,
            used: 0,
            id,
            generation: 1,
        })
    }

    pub fn reset(&mut self) -> Result<(), ArenaError> {
        let next = self
            .generation
            .checked_add(1)
            .ok_or(ArenaError::GenerationExhausted)?;
        self.generation = next;
        self.used = 0;
        Ok(())
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn used(&self) -> usize {
        self.used
    }

    /// Initialize Copy values in unused storage. Failure preserves existing blocks.
    pub fn allocate<T: Copy>(&mut self, len: usize, value: T) -> Result<Block<T>, ArenaError> {
        let size = size_of::<T>();
        if size == 0 {
            return Err(ArenaError::ZeroSizedType);
        }
        let bytes = size.checked_mul(len).ok_or(ArenaError::OutOfSpace)?;
        let address = self.storage.as_ptr() as usize + self.used;
        let padding = address.wrapping_neg() & (align_of::<T>() - 1);
        let offset = if len == 0 {
            0
        } else {
            self.used
                .checked_add(padding)
                .ok_or(ArenaError::OutOfSpace)?
        };
        let end = offset.checked_add(bytes).ok_or(ArenaError::OutOfSpace)?;
        if end > self.capacity {
            return Err(ArenaError::OutOfSpace);
        }
        if len != 0 {
            // SAFETY: alignment includes the actual base address, end is in the
            // allocation, and these bytes have not been exposed by another block.
            let pointer = unsafe { self.storage.as_ptr().add(offset).cast::<T>() };
            for index in 0..len {
                unsafe { pointer.add(index).write(value) };
            }
            self.used = end;
        }
        Ok(Block {
            arena: self.id,
            generation: self.generation,
            offset,
            len,
            element: PhantomData,
        })
    }

    fn pointer<T: Copy>(&self, block: Block<T>) -> Option<*mut T> {
        if block.arena != self.id || block.generation != self.generation {
            return None;
        }
        if block.len == 0 {
            Some(NonNull::<T>::dangling().as_ptr())
        } else {
            // SAFETY: private block fields were created from an initialized,
            // aligned range in this allocation in the current generation.
            Some(unsafe { self.storage.as_ptr().add(block.offset).cast::<T>() })
        }
    }

    pub fn get<T: Copy>(&self, block: Block<T>) -> Option<&[T]> {
        let pointer = self.pointer(block)?;
        // SAFETY: pointer() validates ownership and generation. The borrow of
        // self prevents reset, mutation and destruction while this slice lives.
        Some(unsafe { std::slice::from_raw_parts(pointer, block.len) })
    }

    pub fn get_mut<T: Copy>(&mut self, block: Block<T>) -> Option<&mut [T]> {
        let pointer = self.pointer(block)?;
        // SAFETY: the exclusive arena borrow prevents other slices, allocation
        // or reset; pointer() admits only initialized current-generation ranges.
        Some(unsafe { std::slice::from_raw_parts_mut(pointer, block.len) })
    }
}

impl Drop for FrameArena {
    fn drop(&mut self) {
        // SAFETY: storage was allocated with this exact layout. Copy elements
        // have no destructors and all slices require a live arena borrow.
        unsafe { dealloc(self.storage.as_ptr(), self.layout) };
    }
}
