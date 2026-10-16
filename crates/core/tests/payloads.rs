use qa_core::payloads::{PayloadQueue, QueueError};

#[test]
fn truncate_releases_wrapped_tail_reservations_without_moving_the_prefix() {
    let mut queue = PayloadQueue::load(4, 12).unwrap();
    queue.push(1, b"aaaaa", 0).unwrap();
    queue.push(2, b"bbbb", 0).unwrap();
    assert_eq!(queue.pop(), Some((1, b"aaaaa".as_slice())));
    queue.push(3, b"ccccc", 0).unwrap();
    queue.push(4, b"", 0).unwrap();
    let retained = queue.front().unwrap().1.as_ptr();
    queue.truncate(1);
    assert_eq!(queue.len(), 1);
    assert_eq!(queue.front(), Some((2, b"bbbb".as_slice())));
    assert_eq!(queue.front().unwrap().1.as_ptr(), retained);
    // Eight free bytes still require the three-byte wrap reservation. Starting
    // an eight-byte record at zero would overwrite the retained prefix.
    assert_eq!(queue.push(5, b"12345678", 0), Err(QueueError::PayloadFull));
    queue.push(5, b"ddddd", 0).unwrap();
    assert_eq!(queue.pop(), Some((2, b"bbbb".as_slice())));
    assert_eq!(queue.pop(), Some((5, b"ddddd".as_slice())));
    assert!(queue.is_empty());
    queue.push(6, b"abcdefghijkl", 0).unwrap();
    assert_eq!(queue.pop(), Some((6, b"abcdefghijkl".as_slice())));
}

#[test]
fn truncate_handles_wrapped_header_slots_zero_payloads_and_closed_queues() {
    let mut closed = PayloadQueue::<u32>::default();
    closed.truncate(0);
    closed.truncate(usize::MAX);
    assert!(closed.is_empty());
    let mut queue = PayloadQueue::load(3, 8).unwrap();
    queue.push(1, b"abc", 0).unwrap();
    queue.push(2, b"def", 0).unwrap();
    assert_eq!(queue.pop(), Some((1, b"abc".as_slice())));
    queue.push(3, b"ghi", 0).unwrap();
    queue.truncate(usize::MAX);
    assert_eq!(queue.len(), 2);
    assert_eq!(queue.get(1), Some((3, b"ghi".as_slice())));
    queue.truncate(1);
    queue.push(9, b"", 0).unwrap();
    queue.push(10, b"", 0).unwrap();
    queue.truncate(2);
    queue.push(11, b"jkl", 0).unwrap();
    assert_eq!(queue.pop(), Some((2, b"def".as_slice())));
    assert_eq!(queue.pop(), Some((9, b"".as_slice())));
    assert_eq!(queue.pop(), Some((11, b"jkl".as_slice())));
    queue.push(12, b"abcdefgh", 0).unwrap();
    queue.truncate(0);
    assert!(queue.is_empty());
    queue.push(13, b"ijklmnop", 0).unwrap();
    assert_eq!(queue.pop(), Some((13, b"ijklmnop".as_slice())));
}
