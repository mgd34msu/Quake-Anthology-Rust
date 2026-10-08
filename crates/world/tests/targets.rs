use qa_core::{
    names::{NameMatch, NameTable},
    primitives::{ModuleId, NameId},
};
use qa_world::entities::AllocationPolicy;
use qa_world::entities::EntityTable;
use qa_world::targets::TargetIndex;

#[test]
fn target_iteration_preserves_edict_order_and_rejects_freed_lifetimes() {
    let mut table = EntityTable::new(8, 1).unwrap();
    let names = NameTable::load([b"a".as_slice(), b"b", b"door", b"exit"]).unwrap();
    let first = table
        .allocate(0.0, ModuleId(0), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    let other = table
        .allocate(0.0, ModuleId(0), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    let last = table
        .allocate(0.0, ModuleId(0), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    table.set_targetname(first, Some(NameId(3)));
    table.set_targetname(other, Some(NameId(4)));
    table.set_targetname(last, Some(NameId(3)));
    let mut targets = TargetIndex::new(&table);
    assert!(targets.refresh(&table, &names));
    assert!(!targets.refresh(&table, &names));
    assert_eq!(
        targets
            .find(NameId(3), NameMatch::Exact, &names)
            .collect::<Vec<_>>(),
        [first, last]
    );
    table.release(first, 0.0);
    assert!(targets.refresh(&table, &names));
    assert_eq!(
        targets
            .find(NameId(3), NameMatch::Exact, &names)
            .collect::<Vec<_>>(),
        [last]
    );
    let reused = table
        .allocate(0.0, ModuleId(0), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    table.set_targetname(reused, Some(NameId(3)));
    assert!(targets.refresh(&table, &names));
    assert_eq!(
        targets
            .find(NameId(3), NameMatch::Exact, &names)
            .collect::<Vec<_>>(),
        [reused, last]
    );
    table.set_targetname(last, Some(NameId(4)));
    assert!(targets.refresh(&table, &names));
    assert_eq!(
        targets
            .find(NameId(4), NameMatch::Exact, &names)
            .collect::<Vec<_>>(),
        [other, last]
    );
    assert!(!table.set_targetname(first, Some(NameId(3))));
}

#[test]
fn caller_selects_exact_or_folded_matches_over_one_entity_table() {
    let names = NameTable::load([b"Door".as_slice(), b"door", b"DOOR"]).unwrap();
    let mut table = EntityTable::new(8, 1).unwrap();
    let mut ids = Vec::new();
    for (module, text) in [(1, b"Door".as_slice()), (2, b"door"), (3, b"DOOR")] {
        let id = table
            .allocate(0.0, ModuleId(module), AllocationPolicy::EDICT)
            .unwrap()
            .id;
        table.set_targetname(id, names.find(text));
        ids.push(id);
    }
    let mut index = TargetIndex::new(&table);
    index.refresh(&table, &names);
    let door = names.find(b"door").unwrap();
    assert_eq!(
        index
            .find(door, NameMatch::Exact, &names)
            .collect::<Vec<_>>(),
        [ids[1]]
    );
    assert_eq!(
        index
            .find(door, NameMatch::Folded, &names)
            .collect::<Vec<_>>(),
        ids
    );
    assert_eq!(
        index
            .find(names.find(b"Door").unwrap(), NameMatch::Exact, &names)
            .collect::<Vec<_>>(),
        [ids[0]]
    );
    assert!(table.set_targetname(ids[0], None));
    assert!(index.refresh(&table, &names));
    assert_eq!(
        index
            .find(door, NameMatch::Folded, &names)
            .collect::<Vec<_>>(),
        ids[1..]
    );
}

#[test]
fn empty_target_strings_are_distinct_from_missing_fields() {
    let names = NameTable::load(std::iter::empty()).unwrap();
    let mut table = EntityTable::new(8, 1).unwrap();
    let missing = table
        .allocate(0.0, ModuleId(2), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    let empty = table
        .allocate(0.0, ModuleId(1), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    assert_eq!(table.columns.targetname(missing.slot as usize), None);
    table.set_targetname(empty, Some(NameId(0)));
    let mut index = TargetIndex::new(&table);
    index.refresh(&table, &names);
    for matching in [NameMatch::Exact, NameMatch::Folded] {
        assert_eq!(
            index.find(NameId(0), matching, &names).collect::<Vec<_>>(),
            [empty]
        );
    }
    table.set_targetname(empty, None);
    index.refresh(&table, &names);
    assert!(
        index
            .find(NameId(0), NameMatch::Exact, &names)
            .next()
            .is_none()
    );
}
