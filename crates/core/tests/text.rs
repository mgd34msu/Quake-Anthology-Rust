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
