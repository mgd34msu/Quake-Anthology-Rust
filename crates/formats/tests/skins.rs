use qa_formats::{FormatError, model::skin::SkinMappings};

#[test]
fn comma_skin_reader_keeps_native_comments_tags_quotes_and_duplicate_order() {
    let bytes = b"// heading\n/* block */ tag_weapon,\n BODY,textures/first\n body,textures/second\n\"head\",\"textures/head,detail\"\n";
    let rows = SkinMappings::new(bytes)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        rows.iter().map(|row| row.surface).collect::<Vec<_>>(),
        [b"BODY".as_slice(), b"body", b"head"]
    );
    assert_eq!(
        rows.iter().map(|row| row.shader).collect::<Vec<_>>(),
        [
            b"textures/first".as_slice(),
            b"textures/second",
            b"textures/head,detail"
        ]
    );
    // CommaParse consumes a leading comma as part of a word when separated
    // from the surface by whitespace; it is not a general CSV parser.
    let row = SkinMappings::new(b"body ,shader").next().unwrap().unwrap();
    assert_eq!(row.shader, b",shader");
}

#[test]
fn skin_reader_obeys_native_surface_and_token_limits() {
    let mut bytes = vec![b'a'; 70];
    bytes.extend_from_slice(b",shader\nunused,tail\0ignored,other");
    let rows = SkinMappings::new(&bytes)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].surface, &[b'a'; 63]);
    assert!(SkinMappings::new(&[b'a'; 1024]).next().is_none());
    assert_eq!(
        SkinMappings::new(b"\"unfinished").next().unwrap().err(),
        Some(FormatError::Truncated)
    );
}
