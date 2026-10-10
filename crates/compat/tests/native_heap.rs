use qa_compat::memory::{Heap, MemoryError, ModuleMemory};
#[test]
fn native_tags_share_blocks_keep_crt_allocations_and_survive_reallocation() {
    let base = 4096;
    let mut heap = Heap::load(base, 512, 32).unwrap();
    let mut memory = ModuleMemory::load(base, 512, &[]).unwrap();
    let crt = heap.allocate(16).unwrap();
    let zero = heap.allocate_tagged(16, 0).unwrap();
    let game = heap.allocate_tagged(16, 765).unwrap();
    let level = heap.allocate_tagged(16, -9).unwrap();
    memory.write(game, b"preserved").unwrap();
    let game = heap
        .reallocate(|to, from, bytes| memory.copy(to, from, bytes), game, 64)
        .unwrap();
    assert_eq!(memory.read(game, 9).unwrap(), b"preserved");
    assert_eq!(heap.free_tag(0), 1);
    assert_eq!(heap.free(zero), Err(MemoryError));
    assert_eq!(heap.free_tag(765), 1);
    assert_eq!(heap.free(game), Err(MemoryError));
    assert_eq!(heap.free_tag(765), 0);
    heap.free(crt).unwrap();
    heap.free(level).unwrap();
    assert_eq!(heap.allocate(512), Some(base));
}
#[test]
fn bulk_tag_free_coalesces_in_one_pass_around_live_allocations() {
    let base = 4096;
    let mut heap = Heap::load(base, 1024, 64).unwrap();
    let allocations = (0..32)
        .map(|i| heap.allocate_tagged(16, i % 2).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(heap.free_tag(1), 16);
    for (index, &address) in allocations.iter().enumerate() {
        if index % 2 == 1 {
            assert_eq!(heap.free(address), Err(MemoryError));
        }
    }
    assert_eq!(heap.free_tag(0), 16);
    assert_eq!(heap.allocate(1024), Some(base));
}
#[test]
fn bounded_blocks_align_reuse_coalesce_and_reject_non_allocations() {
    let mut heap = Heap::load(0x1000, 128, 8).unwrap();
    let first = heap.allocate(17).unwrap();
    let second = heap.allocate(1).unwrap();
    let third = heap.allocate(31).unwrap();
    assert_eq!((first, second, third), (0x1000, 0x1020, 0x1030));
    assert_eq!(heap.free(second + 1), Err(MemoryError));
    heap.free(second).unwrap();
    assert_eq!(heap.free(second), Err(MemoryError));
    heap.free(third).unwrap();
    heap.free(first).unwrap();
    assert_eq!(heap.allocate(128), Some(first));
    assert_eq!(heap.allocate(0), None);
    heap.free(first).unwrap();
    assert_eq!(heap.allocate(usize::MAX), None);
    assert_eq!(heap.allocate(0), Some(first));
    heap.free(0).unwrap();
}
#[test]
fn seeded_allocations_keep_live_ranges_disjoint_and_bytes_intact() {
    let mut heap = Heap::load(0x1000, 16384, 128).unwrap();
    let mut memory = ModuleMemory::load(0x1000, 16384, &[]).unwrap();
    let mut live = [None; 64];
    let mut seed = 1729u32;
    for _ in 0..4096 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let slot = ((seed >> 16) as usize) % live.len();
        if let Some((address, bytes)) = live[slot].take() {
            assert!(
                memory
                    .read(address, bytes)
                    .unwrap()
                    .iter()
                    .all(|&b| b == slot as u8)
            );
            heap.free(address).unwrap();
        } else {
            let bytes = (seed as usize & 511) + 1;
            if let Some(address) = heap.allocate(bytes) {
                for &(other, size) in live.iter().flatten() {
                    assert!(address + bytes as u64 <= other || other + size as u64 <= address);
                }
                memory.read_mut(address, bytes).unwrap().fill(slot as u8);
                live[slot] = Some((address, bytes));
            }
        }
    }
    for (address, bytes) in live.into_iter().flatten() {
        assert!(
            memory
                .read(address, bytes)
                .unwrap()
                .iter()
                .all(|&b| b == memory.read(address, 1).unwrap()[0])
        );
        heap.free(address).unwrap();
    }
    assert_eq!(heap.allocate(16384), Some(0x1000));
}
#[test]
fn metadata_limit_uses_existing_extent_and_reallocation_preserves_old_on_failure() {
    let base = 1 << 40;
    let mut heap = Heap::load(base, 128, 2).unwrap();
    let mut memory = ModuleMemory::load(base, 128, &[]).unwrap();
    let first = heap.allocate(17).unwrap();
    let second = heap.allocate(16).unwrap();
    memory.write(first, b"original").unwrap();
    assert_eq!(
        heap.reallocate(|to, from, bytes| memory.copy(to, from, bytes), first, 64)
            .unwrap(),
        0
    );
    assert_eq!(memory.read(first, 8).unwrap(), b"original");
    heap.free(second).unwrap();
    let moved = heap
        .reallocate(|to, from, bytes| memory.copy(to, from, bytes), first, 64)
        .unwrap();
    assert_ne!(moved, first);
    assert_eq!(memory.read(moved, 8).unwrap(), b"original");
    assert_eq!(
        heap.reallocate(|to, from, bytes| memory.copy(to, from, bytes), moved, 4)
            .unwrap(),
        moved
    );
    assert_eq!(
        heap.reallocate(|to, from, bytes| memory.copy(to, from, bytes), moved, 0)
            .unwrap(),
        0
    );
    assert_eq!(heap.allocate(128), Some(base));
}
