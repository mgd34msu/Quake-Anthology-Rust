use qa_core::rle::{encode_pvs, pvs};

#[test]
fn native_zero_run_boundaries_preserve_literals_and_append_position() {
    for (length, runs) in [
        (0, vec![]),
        (1, vec![0, 1]),
        (254, vec![0, 254]),
        (255, vec![0, 255]),
        (256, vec![0, 255, 0, 1]),
        (510, vec![0, 255, 0, 255]),
        (511, vec![0, 255, 0, 255, 0, 1]),
    ] {
        let mut row = vec![3];
        row.resize(length + 1, 0);
        row.push(4);
        let mut encoded = vec![99];
        encode_pvs(&row, &mut encoded);
        let mut expected = vec![99, 3];
        expected.extend(runs);
        expected.push(4);
        assert_eq!(encoded, expected, "zero count {length}");
        let mut decoded = vec![255; row.len()];
        pvs(&encoded, 1, row.len(), Some(&mut decoded)).unwrap();
        assert_eq!(decoded, row);
    }
    let mut encoded = vec![99];
    encode_pvs(&[], &mut encoded);
    assert_eq!(encoded, [99]);
}

#[test]
fn every_native_byte_round_trips_without_changing_nonzero_literals() {
    for row in [
        (0..=255).collect::<Vec<u8>>(),
        (0..=255).rev().collect::<Vec<u8>>(),
    ] {
        let mut encoded = Vec::new();
        encode_pvs(&row, &mut encoded);
        assert_eq!(encoded.len(), 257);
        assert!(encoded.windows(2).any(|pair| pair == [0, 1]));
        let mut decoded = vec![0; row.len()];
        pvs(&encoded, 0, row.len(), Some(&mut decoded)).unwrap();
        assert_eq!(decoded, row);
    }
}

#[test]
fn appended_rows_keep_independent_offsets_and_lengths() {
    let rows = [vec![0; 300], vec![1, 0, 2, 0, 3], vec![], vec![255; 9]];
    let mut encoded = Vec::new();
    let mut offsets = Vec::new();
    for row in &rows {
        offsets.push(encoded.len());
        encode_pvs(row, &mut encoded);
    }
    assert_eq!(offsets, [0, 4, 11, 11]);
    for (row, offset) in rows.iter().zip(offsets) {
        let mut decoded = vec![17; row.len()];
        pvs(&encoded, offset, row.len(), Some(&mut decoded)).unwrap();
        assert_eq!(&decoded, row);
    }
}
