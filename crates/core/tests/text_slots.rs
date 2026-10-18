use qa_core::text::{SlotsError, SlotsLimits, TextSlots};

fn limits() -> SlotsLimits {
    SlotsLimits {
        rows: 3,
        width: 8,
        bytes: 12,
    }
}

#[test]
fn independent_rows_preserve_addresses_and_rejected_replacements() {
    let mut rows = TextSlots::load([3, 0, 8], limits()).unwrap();
    let first = rows.get(0).unwrap().as_ptr();
    let last = rows.get(2).unwrap().as_ptr();
    assert!(rows.set(0, b"a\0\xff").unwrap());
    assert!(rows.set(2, "é".as_bytes()).unwrap());
    assert_eq!(rows.text(0), None);
    assert_eq!(rows.text(2), Some("é"));
    assert_eq!(rows.set(0, b"long"), Err(SlotsError::Capacity));
    assert_eq!(rows.get(0), Some(b"a\0\xff".as_slice()));
    assert!(!rows.set(0, b"a\0\xff").unwrap());
    assert_eq!(rows.set(1, b"x"), Err(SlotsError::Capacity));
    assert!(!rows.set(1, b"").unwrap());
    assert_eq!(rows.set(3, b""), Err(SlotsError::Index));
    assert_eq!(rows.write_parts(2, &[b"123", b"456789"]), Ok((8, true)));
    assert_eq!(rows.get(2), Some(b"12345678".as_slice()));
    assert_eq!(rows.get(0).unwrap().as_ptr(), first);
    assert_eq!(rows.get(2).unwrap().as_ptr(), last);
    assert_eq!(rows.set_length(2, 9), Err(SlotsError::Capacity));
    assert_eq!(rows.get(2), Some(b"12345678".as_slice()));
    rows.set_length(2, 0).unwrap();
    assert!(rows.get(2).unwrap().is_empty());
    assert_eq!(rows.get(0), Some(b"a\0\xff".as_slice()));
}

#[test]
fn limits_reject_bad_layouts_before_allocating_the_byte_buffer() {
    for capacities in [vec![1, 1, 1, 1], vec![9], vec![8, 8]] {
        assert!(matches!(
            TextSlots::load(capacities, limits()),
            Err(SlotsError::Capacity)
        ));
    }
    assert!(matches!(
        TextSlots::load(
            [u32::MAX as usize, 1],
            SlotsLimits {
                rows: 2,
                width: usize::MAX,
                bytes: usize::MAX,
            }
        ),
        Err(SlotsError::Capacity)
    ));
    let empty = TextSlots::load(
        [],
        SlotsLimits {
            rows: 0,
            width: 0,
            bytes: 0,
        },
    )
    .unwrap();
    assert!(empty.is_empty());
    assert_eq!(empty.get(0), None);
}
