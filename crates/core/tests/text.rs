use qa_core::text::FixedText;
use std::fmt::Write;

#[test]
fn byte_replacement_preserves_native_text_and_rejects_growth_atomically() {
    let mut text = FixedText::<8>::default();
    text.set_bytes(b"A\x80BC\xfe").unwrap();
    text.replace_bytes(1..3, b"\xfd\x82Z").unwrap();
    assert_eq!(text.as_bytes(), b"A\xfd\x82ZC\xfe");
    let before = text.clone();
    assert!(text.replace_bytes(2..3, b"1234").is_err());
    assert_eq!(text.as_bytes(), before.as_bytes());
    text.replace_bytes(1..4, b"Q").unwrap();
    assert_eq!(text.as_bytes(), b"AQC\xfe");
    text.replace_bytes(4..4, b"!").unwrap();
    assert_eq!(text.as_bytes(), b"AQC\xfe!");
    text.replace_bytes(0..5, b"ok").unwrap();
    text.replace(1..2, "é").unwrap();
    assert_eq!(text.as_str(), "oé");
}

#[test]
fn info_key_and_value_limits_are_independent_and_validate_before_removal() {
    use qa_core::text::{InfoBytes, InfoError, InfoRules, info_set};
    for value_bytes in [64, 256] {
        let rules = InfoRules {
            bytes: 2048,
            key_bytes: 64,
            value_bytes,
            filter: InfoBytes::Ascii7Printable,
        };
        let mut buffer = [0; 2048];
        let mut len = 0;
        let key = [b'k'; 63];
        let value = vec![b'v'; value_bytes - 1];
        info_set(&mut buffer, &mut len, &key, &value, rules).unwrap();
        let before = buffer;
        assert_eq!(
            info_set(&mut buffer, &mut len, &key, &vec![b'v'; value_bytes], rules),
            Err(InfoError::Invalid)
        );
        assert_eq!(buffer, before);
        assert_eq!(
            info_set(&mut buffer, &mut len, &[b'k'; 64], b"value", rules),
            Err(InfoError::Invalid)
        );
        assert_eq!(buffer, before);
    }
}

#[test]
fn info_writer_keeps_native_validation_filtering_and_failed_replacement_rules() {
    use qa_core::text::{InfoBytes, InfoError, InfoRules, info_set};
    let mut buffer = [0xcc; 64];
    let original = b"\\name\\old\\name\\later\\keep\\raw\x80";
    buffer[..original.len()].copy_from_slice(original);
    let mut len = original.len();
    let native = InfoRules {
        bytes: 64,
        key_bytes: 64,
        value_bytes: 64,
        filter: InfoBytes::Ascii7Printable,
    };
    assert_eq!(
        info_set(&mut buffer, &mut len, b"name", b"bad\xdc", native),
        Err(InfoError::Invalid)
    );
    assert_eq!(&buffer[..len], original);
    info_set(&mut buffer, &mut len, b"name", b"\xe1\x01\xff\x80b", native).unwrap();
    assert_eq!(&buffer[..len], b"\\keep\\raw\x80\\name\\ab");
    assert_eq!(buffer[len], 0);
    assert_eq!(
        info_set(
            &mut buffer,
            &mut len,
            b"name",
            b"long replacement",
            InfoRules {
                bytes: 16,
                ..native
            }
        ),
        Err(InfoError::Capacity)
    );
    assert_eq!(&buffer[..len], b"\\keep\\raw\x80");
    assert_eq!(buffer[len], 0);
    info_set(&mut buffer, &mut len, b"keep", b"", native).unwrap();
    assert_eq!(len, 0);
    let exact = InfoRules {
        filter: InfoBytes::Exact,
        ..native
    };
    info_set(&mut buffer, &mut len, b"utf8", "café".as_bytes(), exact).unwrap();
    assert_eq!(&buffer[..len], "\\utf8\\café".as_bytes());
    let prior = buffer;
    assert_eq!(
        info_set(&mut buffer, &mut len, b"bad", b"\x01", exact),
        Err(InfoError::Invalid)
    );
    assert_eq!(buffer, prior);
}

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

#[test]
fn string_boundaries_preserve_raw_bytes_and_require_native_termination() {
    use qa_core::text::{terminated, until_nul};
    for (bytes, expected) in [
        (b"\xff\x80\0tail".as_slice(), b"\xff\x80".as_slice()),
        (b"\0tail", b""),
        (b"plain", b"plain"),
        (b"", b""),
    ] {
        assert_eq!(until_nul(bytes), expected);
        assert_eq!(terminated(bytes).is_some(), bytes.contains(&0));
    }
}

#[test]
fn integer_prefix_accepts_raw_suffixes_and_saturates_native_signed_limits() {
    use qa_core::text::atoi_prefix;
    for (bytes, expected) in [
        (&b" \t\r\n\x0b\x0c-5tail"[..], -5),
        (&b"+123\xff"[..], 123),
        (&b"214748364999"[..], i32::MAX),
        (&b"-214748364999"[..], i32::MIN),
        (&b"-"[..], 0),
        (&b"\xff12"[..], 0),
        (&b""[..], 0),
    ] {
        assert_eq!(atoi_prefix(bytes), expected, "{bytes:?}");
    }
}
