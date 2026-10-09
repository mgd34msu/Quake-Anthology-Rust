use qa_core::names::{NameMatch, NameTable, NamesError, canonical_path, compare_folded};
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

#[test]
fn registrations_preserve_loaded_ids_and_group_ids_across_sort_insertions() {
    let mut names = NameTable::load_reserved([b"middle".as_slice(), b"z"], 4, 16).unwrap();
    let middle = names.find(b"middle").unwrap();
    let end = names.find(b"z").unwrap();
    let before = names.intern(b"a").unwrap();
    let upper = names.intern(b"MIDDLE").unwrap();
    assert_ne!(upper, middle);
    assert_eq!(names.folded(upper), Some(middle));
    assert_eq!(names.find(b"MIDDLE"), Some(upper));
    assert_eq!(names.find_folded(b"MiDdLe"), Some(middle));
    assert_eq!(names.get(middle), Some(b"middle".as_slice()));
    assert_eq!(names.get(end), Some(b"z".as_slice()));
    assert_eq!(names.find(b"a"), Some(before));
    assert_eq!(names.intern(b"a").unwrap(), before);
    assert_eq!(names.find(b""), Some(NameId(0)));
}

#[test]
fn fixed_registration_storage_rejects_without_corrupting_prior_names() {
    let mut names = NameTable::load_reserved([], 1, 2).unwrap();
    assert_eq!(names.intern(b"abc"), Err(NamesError::Capacity));
    let id = names.intern(b"ab").unwrap();
    assert_eq!(names.intern(b"c"), Err(NamesError::Capacity));
    assert_eq!(names.get(id), Some(b"ab".as_slice()));
    assert_eq!(names.find_folded(b"AB"), Some(id));
    assert_eq!(names.len(), 2);
}

#[test]
fn path_keys_share_one_conversion_without_changing_exact_name_rules() {
    let mut names = NameTable::load_reserved([], 4, 128).unwrap();
    let id = names.intern_path("Textures\\WALL.Ä.TGA").unwrap();
    assert_eq!(names.get(id), Some("textures/wall.Ä.tga".as_bytes()));
    assert_eq!(names.intern_path("TEXTURES/wall.Ä.tga").unwrap(), id);
    assert_eq!(names.find_path("textures\\Wall.Ä.TGA"), Some(id));
    assert_eq!(names.find_path("textures/wall.ä.tga"), None);
    assert_eq!(canonical_path("A\\B.Ä"), "a/b.Ä");
    let raw = names.intern(b"A\\B").unwrap();
    let slash = names.intern(b"a/b").unwrap();
    assert_ne!(names.folded(raw), names.folded(slash));
    assert_eq!(compare_folded(b"B", b"a"), std::cmp::Ordering::Greater);
    assert_eq!(compare_folded(b"A", b"a"), std::cmp::Ordering::Equal);
    assert_eq!(compare_folded(b"z", b"_"), std::cmp::Ordering::Less);
}

#[test]
fn path_registration_does_not_reuse_or_merge_a_raw_case_variant() {
    let mut names = NameTable::load_reserved([b"WALL".as_slice()], 1, 4).unwrap();
    let raw = names.find(b"WALL").unwrap();
    assert_eq!(names.find_path("Wall"), None);
    let path = names.intern_path("Wall").unwrap();
    assert_ne!(raw, path);
    assert_eq!(names.get(raw), Some(b"WALL".as_slice()));
    assert_eq!(names.get(path), Some(b"wall".as_slice()));
    assert_eq!(names.find_path("WALL"), Some(path));
    assert_eq!(names.find_folded(b"wall"), Some(raw));
}
