use qa_core::text::FixedText;
use std::fmt::Write;

#[test]
fn append_keeps_raw_bytes_and_rejects_overflow_without_changing_text() {
    let mut text = FixedText::<6>::default();
    text.write_str("a").unwrap();
    text.append_bytes(b"\x82b").unwrap();
    assert_eq!(text.as_bytes(), b"a\x82b");
    assert!(text.append_bytes(b"long").is_err());
    assert_eq!(text.as_bytes(), b"a\x82b");
    text.append_bytes(b"123").unwrap();
    assert_eq!(text.as_bytes(), b"a\x82b123");
}
#[test]
fn native_info_values_preserve_first_match_empty_fields_and_caller_case_rules() {
    use qa_core::text::info_value;
    let info = b"\\name\\Mike\\empty\\\\name\\later\\binary\\\xff\x80";
    assert_eq!(&info[info_value(info, b"name", false).unwrap()], b"Mike");
    assert_eq!(&info[info_value(info, b"empty", false).unwrap()], b"");
    assert_eq!(&info[info_value(info, b"NAME", true).unwrap()], b"Mike");
    assert_eq!(info_value(info, b"NAME", false), None);
    assert_eq!(info_value(info, b"missing", false), None);
    assert_eq!(
        &info[info_value(info, b"binary", false).unwrap()],
        b"\xff\x80"
    );
    assert_eq!(info_value(b"name\\Mike", b"name", false), Some(5..9));
    assert_eq!(info_value(b"\\dangling", b"dangling", false), None);
    assert_eq!(info_value(b"\\key\\", b"key", false), Some(5..5));
    assert_eq!(info_value(b"", b"", false), None);
}

#[test]
fn native_info_removal_keeps_unmatched_bytes_and_native_duplicate_rules() {
    use qa_core::text::info_remove;
    for (source, key, folded, all, expected) in [
        (
            b"\\name\\Mike\\empty\\\\name\\later\\binary\\\xff\x80".as_slice(),
            b"name".as_slice(),
            false,
            true,
            b"\\empty\\\\binary\\\xff\x80".as_slice(),
        ),
        (
            b"\\name\\first\\name\\second",
            b"name",
            false,
            false,
            b"\\name\\second",
        ),
        (
            b"\\name\\first\\NAME\\second",
            b"NAME",
            false,
            true,
            b"\\name\\first",
        ),
        (b"\\name\\first\\NAME\\second", b"NAME", true, true, b""),
        (b"name\\Mike\\empty\\", b"name", false, true, b"\\empty\\"),
        (b"\\\\value\\tail", b"", false, true, b"\\tail"),
        (
            b"\\name\\Mike\\dangling",
            b"missing",
            false,
            true,
            b"\\name\\Mike\\dangling",
        ),
        (
            b"\\name\\Mike\\dangling",
            b"name",
            false,
            true,
            b"\\dangling",
        ),
        (b"", b"", false, true, b""),
    ] {
        let mut bytes = source.to_vec();
        let len = info_remove(&mut bytes, key, folded, all);
        assert_eq!(&bytes[..len], expected);
    }
}
