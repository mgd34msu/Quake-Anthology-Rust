use qa_core::names::NameTable;
use qa_core::primitives::NameId;

#[test]
fn names_keep_first_spelling_and_match_ascii_case_without_text_conversion() {
    let names = NameTable::load([
        b"Monster_Ogre".as_slice(),
        b"monster_ogre",
        b"door",
        b"",
        b"raw\xff",
    ])
    .unwrap();
    let ogre = names.find(b"MONSTER_OGRE").unwrap();
    assert_eq!(names.get(ogre), Some(b"Monster_Ogre".as_slice()));
    assert_eq!(names.find(b""), Some(NameId(0)));
    assert_eq!(names.len(), 4);
    assert!(names.find(b"unknown").is_none());
    assert!(names.get(NameId(u32::MAX)).is_none());
    assert_eq!(
        names.get(names.find(b"RAW\xff").unwrap()),
        Some(b"raw\xff".as_slice())
    );
}
