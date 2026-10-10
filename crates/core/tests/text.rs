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
