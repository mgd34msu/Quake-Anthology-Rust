use qa_core::bits::BitSet;

#[test]
fn membership_scans_respect_capacity_and_retain_storage() {
    for len in [0, 1, 63, 64, 65, 127, 128, 129, 8192] {
        let mut full = BitSet::filled(len);
        assert_eq!(full.len(), len);
        assert_eq!(full.words().len(), len.div_ceil(64));
        assert_eq!(full.next_clear(0), None);
        assert_eq!(full.next_set(len), None);
        for start in 0..len {
            assert_eq!(full.next_set(start), Some(start));
        }
        assert!(!full.contains(len));
        let pointer = full.words().as_ptr();
        full.clear();
        assert_eq!(full.words().as_ptr(), pointer);
        assert_eq!(full.next_set(0), None);
        assert_eq!(full.next_clear(0), (len != 0).then_some(0));
        assert!(!full.insert(len));
        assert!(!full.test_and_clear(usize::MAX));
    }
}

#[test]
fn seeded_updates_and_word_scans_match_independent_membership_model() {
    let mut seed = 3367u32;
    for len in [1, 65, 129, 8192] {
        let mut bits = BitSet::empty(len);
        let mut model = vec![false; len];
        for _ in 0..600 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let index = seed as usize % len;
            let value = seed & 32 != 0;
            let changed = if value {
                bits.insert(index)
            } else {
                bits.test_and_clear(index)
            };
            assert_eq!(changed, model[index] != value);
            model[index] = value;
            assert_eq!(bits.contains(index), value);
            let start = (seed >> 8) as usize % (len + 1);
            assert_eq!(bits.next_set(start), (start..len).find(|&i| model[i]));
            assert_eq!(bits.next_clear(start), (start..len).find(|&i| !model[i]));
        }
    }
}
