use qa_core::names::{NameMatch, NameTable};
use qa_core::primitives::NameId;

#[test]
fn exact_names_retain_case_variants_and_folded_lookup_is_explicit() {
    let names = NameTable::load([
        b"Monster_Ogre".as_slice(),
        b"monster_ogre",
        b"door",
        b"",
        b"raw\xff",
        b"RAW\xff",
    ])
    .unwrap();
    let ogre = names.find(b"Monster_Ogre").unwrap();
    let lower = names.find(b"monster_ogre").unwrap();
    assert_ne!(ogre, lower);
    assert_eq!(names.get(ogre), Some(b"Monster_Ogre".as_slice()));
    assert_eq!(names.get(lower), Some(b"monster_ogre".as_slice()));
    assert_eq!(names.len(), 6);
    for (rule, expected) in [(NameMatch::Exact, None), (NameMatch::Folded, Some(ogre))] {
        let result = match rule {
            NameMatch::Exact => names.find(b"MONSTER_OGRE"),
            NameMatch::Folded => names.find_folded(b"MONSTER_OGRE"),
        };
        assert_eq!(result, expected);
    }
    assert_eq!(names.folded(ogre), Some(ogre));
    assert_eq!(names.folded(lower), Some(ogre));
    assert_eq!(
        names.get(names.find_folded(b"Raw\xff").unwrap()),
        Some(b"RAW\xff".as_slice())
    );
    assert!(names.find(b"unknown").is_none());
    assert!(names.find_folded(b"UNKNOWN").is_none());
    assert!(names.get(NameId(u32::MAX)).is_none());
    assert!(names.folded(NameId(u32::MAX)).is_none());
}

#[test]
fn exact_sorting_and_group_representatives_are_independent_of_input_order() {
    let input = [
        b"zeta".as_slice(),
        b"Alpha",
        b"alpha",
        b"BETA",
        b"beta",
        b"ALPHA",
        b"Alpha",
    ];
    let forward = NameTable::load(input).unwrap();
    let reverse = NameTable::load(input.into_iter().rev()).unwrap();
    let expected = [
        b"".as_slice(),
        b"ALPHA",
        b"Alpha",
        b"BETA",
        b"alpha",
        b"beta",
        b"zeta",
    ];
    assert_eq!(forward.len(), expected.len());
    assert_eq!(reverse.len(), expected.len());
    for (index, bytes) in expected.into_iter().enumerate() {
        let id = NameId(index as u32);
        assert_eq!(forward.get(id), Some(bytes));
        assert_eq!(reverse.get(id), Some(bytes));
        assert_eq!(forward.find(bytes), Some(id));
        assert_eq!(reverse.find(bytes), Some(id));
        assert_eq!(forward.folded(id), reverse.folded(id));
        assert_eq!(forward.find_folded(bytes), forward.folded(id));
    }
    assert_eq!(forward.find_folded(b"aLpHa"), Some(NameId(1)));
    assert_eq!(forward.find_folded(b"BeTa"), Some(NameId(3)));
    assert_eq!(forward.find_folded(b"ZETA"), Some(NameId(6)));
}

#[test]
fn raw_bytes_fold_only_ascii_and_preserve_non_utf8_and_embedded_nuls() {
    let names = NameTable::load([
        b"\xff\0A".as_slice(),
        b"\xff\0a",
        b"\xc0",
        b"\xe0",
        b"A/B",
        b"a/b",
        b"A\\B",
    ])
    .unwrap();
    let upper = names.find(b"\xff\0A").unwrap();
    let lower = names.find(b"\xff\0a").unwrap();
    assert_ne!(upper, lower);
    assert_eq!(names.get(upper), Some(b"\xff\0A".as_slice()));
    assert_eq!(names.get(lower), Some(b"\xff\0a".as_slice()));
    assert_eq!(names.folded(lower), Some(upper));
    assert_eq!(names.find_folded(b"\xff\0a"), Some(upper));
    let high = names.find(b"\xc0").unwrap();
    let other = names.find(b"\xe0").unwrap();
    assert_ne!(names.folded(high), names.folded(other));
    assert_eq!(names.find_folded(b"\xc0"), Some(high));
    assert_eq!(names.find_folded(b"\xe0"), Some(other));
    assert_eq!(names.find_folded(b"a/B"), names.find(b"A/B"));
    assert_eq!(names.find_folded(b"a\\b"), names.find(b"A\\B"));
    assert_ne!(names.find_folded(b"a/b"), names.find_folded(b"a\\b"));
    assert!(names.find_folded(b"\xff\0b").is_none());
}

#[test]
fn empty_name_has_zero_identity_and_a_stable_folded_group() {
    let empty = NameTable::load(std::iter::empty::<&[u8]>()).unwrap();
    assert_eq!(empty.len(), 1);
    assert!(!empty.is_empty());
    assert_eq!(empty.get(NameId(0)), Some(b"".as_slice()));
    assert_eq!(empty.find(b""), Some(NameId(0)));
    assert_eq!(empty.find_folded(b""), Some(NameId(0)));
    assert_eq!(empty.folded(NameId(0)), Some(NameId(0)));
    assert!(empty.find(b"\0").is_none());
    assert!(empty.find_folded(b"\0").is_none());
    assert!(empty.folded(NameId(1)).is_none());

    let repeated = NameTable::load([b"".as_slice(), b"\0", b""]).unwrap();
    assert_eq!(repeated.len(), 2);
    assert_eq!(repeated.find(b""), Some(NameId(0)));
    assert_eq!(repeated.find(b"\0"), Some(NameId(1)));
    assert_eq!(repeated.find_folded(b"\0"), Some(NameId(1)));
}
