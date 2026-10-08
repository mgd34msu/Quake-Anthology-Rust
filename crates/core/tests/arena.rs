use qa_core::arena::{ArenaError, FrameArena};

#[repr(align(128))]
#[derive(Clone, Copy, Debug, PartialEq)]
struct Wide([u32; 32]);

#[test]
fn typed_ranges_are_aligned_initialized_and_reused_without_growing() {
    let mut arena = FrameArena::load(1024).unwrap();
    let bytes = arena.allocate(3, 7u8).unwrap();
    let vectors = arena.allocate(2, Wide([11; 32])).unwrap();
    let slice = arena.get_mut(vectors).unwrap();
    assert_eq!(slice.as_ptr() as usize % 128, 0);
    assert_eq!(slice, &[Wide([11; 32]); 2]);
    slice[1].0[31] = 19;
    assert_eq!(arena.get(bytes).unwrap(), [7; 3]);
    let address = arena.get(bytes).unwrap().as_ptr();
    let used = arena.used();
    assert_eq!(
        arena.allocate(usize::MAX, 0u32).unwrap_err(),
        ArenaError::OutOfSpace
    );
    assert_eq!(
        arena.allocate(1024, 0u8).unwrap_err(),
        ArenaError::OutOfSpace
    );
    assert_eq!(arena.used(), used);
    assert_eq!(arena.get(vectors).unwrap()[1].0[31], 19);
    arena.reset().unwrap();
    assert!(arena.get(bytes).is_none());
    assert!(arena.get_mut(vectors).is_none());
    let reused = arena.allocate(3, 23u8).unwrap();
    assert_eq!(arena.get(reused).unwrap().as_ptr(), address);
    assert_eq!(arena.get(reused).unwrap(), [23; 3]);
    assert_eq!(arena.capacity(), 1024);
}

#[test]
fn cross_arena_handles_and_zero_capacity_are_bounded() {
    let mut first = FrameArena::load(64).unwrap();
    let second = FrameArena::load(64).unwrap();
    let block = first.allocate(4, 42u32).unwrap();
    assert!(second.get(block).is_none());
    let mut empty = FrameArena::load(0).unwrap();
    let zero = empty.allocate(0, 3u64).unwrap();
    assert!(empty.get(zero).unwrap().is_empty());
    assert_eq!(empty.allocate(1, 0u8).unwrap_err(), ArenaError::OutOfSpace);
    assert_eq!(
        empty.allocate(2, ()).unwrap_err(),
        ArenaError::ZeroSizedType
    );
    assert!(FrameArena::load(usize::MAX).is_err());
}
