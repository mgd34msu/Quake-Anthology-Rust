use qa_formats::{FormatError, localization};

#[test]
fn localization_uses_common_comments_and_escaped_quotes_and_skips_platform_rows() {
    let rows = localization::read(
        br#"
        // line
        key = "line\n\t\r\"quoted\"\\\q"
        /* block */ key < xbox ps5 > = "skip"
        raw = "raw"
    "#,
    )
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(&*rows[0].key, b"key");
    assert_eq!(&*rows[0].format, b"line\n\t\r\"quoted\"\\q");
    assert_eq!(&*rows[1].key, b"raw");
    assert_eq!(&*rows[1].format, b"raw");
}

#[test]
fn localization_rejects_incomplete_syntax_and_load_limits() {
    for bytes in [b"key = \"broken".as_slice(), b"key < xbox", b"key"] {
        assert!(matches!(
            localization::read(bytes),
            Err(FormatError::Truncated)
        ));
    }
    assert!(matches!(
        localization::read(b"key : value"),
        Err(FormatError::InvalidValue)
    ));
    let long_key = format!("{} = value", "k".repeat(64));
    assert!(matches!(
        localization::read(long_key.as_bytes()),
        Err(FormatError::InvalidRange)
    ));
    let long_text = format!("key = \"{}\"", "x".repeat(1024));
    assert!(matches!(
        localization::read(long_text.as_bytes()),
        Err(FormatError::InvalidRange)
    ));
}
