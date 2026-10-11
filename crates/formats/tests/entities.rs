use qa_formats::{
    FormatError,
    entities::{EntityLump, EntitySyntax, EntityTokens},
};

#[test]
fn native_syntax_keeps_borrowed_values_case_policy_and_duplicate_order() {
    let text = br#"{ "classname" "worldspawn" "Name" "a" "name" "b" "message" "line\nnext" }"#;
    let q1 = EntityLump::parse(text, EntitySyntax::Quake).unwrap();
    assert_ne!(q1.names.find(b"Name"), q1.names.find(b"name"));
    assert_eq!(q1.fields[3].value, b"line\\nnext");
    assert_eq!(q1.fields[0].value.as_ptr(), text[15..].as_ptr());
    let q2 = EntityLump::parse(text, EntitySyntax::Quake2).unwrap();
    assert_eq!(q2.fields[1].key, q2.fields[2].key);
    assert_eq!(q2.fields[2].value, b"b");
    assert!(EntityLump::parse(b"{\"x\" \"y\"}", EntitySyntax::Quake).is_ok());
    assert!(EntityLump::parse(b"{\"x\" \"y\"}", EntitySyntax::Quake2).is_err());
}

#[test]
fn resumed_entity_tokens_keep_empty_values_raw_bytes_and_caller_syntax() {
    let bytes = b"{ /* comment */ \"key\" \"\" // line\n \"raw\" \"\x82\\n\" }\0ignored";
    let mut reader = EntityTokens::new(bytes, EntitySyntax::Quake3);
    let length = reader.source_length();
    let mut found = Vec::new();
    loop {
        let token = reader.next_token().unwrap();
        let Some(token) = token else { break };
        found.push(token);
        reader = EntityTokens::resume(&bytes[..length], EntitySyntax::Quake3, reader.position())
            .unwrap();
    }
    assert_eq!(
        found,
        [b"{".as_slice(), b"key", b"", b"raw", b"\x82\\n", b"}"]
    );
    assert!(EntityTokens::resume(&bytes[..length], EntitySyntax::Quake3, length + 1).is_err());
    let bytes = b"a:b (x)";
    let collect = |syntax| {
        let mut reader = EntityTokens::new(bytes, syntax);
        let mut result = Vec::new();
        while let Some(token) = reader.next_token().unwrap() {
            result.push(token);
        }
        result
    };
    assert_eq!(
        collect(EntitySyntax::Quake),
        [b"a".as_slice(), b":", b"b", b"(", b"x", b")"]
    );
    assert_eq!(collect(EntitySyntax::Quake3), [b"a:b".as_slice(), b"(x)"]);
}
#[test]
fn q3_value_newlines_match_server_entity_token_calls() {
    let text = b"/* header */ {\n\"classname\"\n\"worldspawn\"\n}\0ignored";
    let q3 = EntityLump::parse(text, EntitySyntax::Quake3).unwrap();
    assert_eq!(q3.records.len(), 1);
    assert_eq!(q3.fields[0].value, b"worldspawn");
    assert!(EntityLump::parse(text, EntitySyntax::Quake2).is_err());
}
#[test]
fn known_fields_leave_typed_values_and_only_unknown_guest_pairs() {
    let lump = EntityLump::parse(
        b"{ \"health\" \"50\" \"health\" \"75\" \"_note\" \"editor\" \"module_extra\" \"raw\" }",
        EntitySyntax::Quake,
    )
    .unwrap();
    let health = lump.names.find(b"health").unwrap();
    let columns = lump
        .convert::<i32>(true, |entry, value| {
            if entry.key != health {
                return Ok(false);
            }
            *value = std::str::from_utf8(entry.value)
                .map_err(|_| FormatError::InvalidValue)?
                .parse()
                .map_err(|_| FormatError::InvalidValue)?;
            Ok(true)
        })
        .unwrap();
    assert_eq!(columns.values, [75]);
    assert_eq!(columns.guest_records.len(), 1);
    assert_eq!(columns.guest_records[0], 0..1);
    assert_eq!(columns.guest_fields[0].value, b"raw");
}
#[test]
fn incomplete_entity_records_never_escape_as_success() {
    for text in [
        b"{ \"key\" }".as_slice(),
        b"{ \"key\"",
        b"{ \"key\" \"value\"",
        b"garbage",
        b"{ \"unfinished",
        b"/* unfinished",
    ] {
        assert!(EntityLump::parse(text, EntitySyntax::Quake3).is_err());
    }
    let mut state = 0x454e5449u32;
    for _ in 0..10000 {
        let mut bytes = b"{ \"classname\" \"worldspawn\" }".to_vec();
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        let i = state as usize % bytes.len();
        bytes[i] ^= (state >> 24) as u8;
        let _ = EntityLump::parse(&bytes, EntitySyntax::Quake3);
    }
}
