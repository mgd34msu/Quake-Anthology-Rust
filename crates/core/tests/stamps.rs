use qa_core::stamps::StampSet;

#[test]
fn membership_and_test_and_set_share_one_epoch() {
    let mut set = StampSet::new(4);
    assert_eq!(set.len(), 4);
    assert!(!set.is_empty());
    assert!((0..set.len()).all(|index| !set.contains(index)));
    set.mark(2);
    assert!(set.contains(2));
    assert!(!set.contains(1));
    assert!(set.test_and_set(2));
    assert!(!set.test_and_set(1));
    assert!(set.contains(1));
    assert!(set.test_and_set(1));
    set.begin();
    assert!((0..set.len()).all(|index| !set.contains(index)));
    assert!(!set.test_and_set(2));
    assert!(set.contains(2));
}

#[test]
fn independent_sets_do_not_retire_each_others_membership() {
    let mut first = StampSet::new(2);
    let mut second = StampSet::new(2);
    first.mark(1);
    assert!(!second.contains(1));
    second.mark(1);
    first.begin();
    assert!(!first.contains(1));
    assert!(second.contains(1));
    second.begin();
    assert!(!second.contains(1));
}

#[test]
fn empty_set_keeps_its_cold_capacity_across_queries() {
    let mut set = StampSet::new(0);
    for _ in 0..3 {
        assert_eq!(set.len(), 0);
        assert!(set.is_empty());
        set.begin();
    }
}
