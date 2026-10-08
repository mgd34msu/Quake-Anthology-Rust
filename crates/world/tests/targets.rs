use qa_core::primitives::{ModuleId, NameId};
use qa_world::entities::AllocationPolicy;
use qa_world::entities::EntityTable;
use qa_world::targets::TargetIndex;

#[test]
fn target_iteration_preserves_edict_order_and_rejects_freed_lifetimes() {
    let mut table = EntityTable::new(8, 1).unwrap();
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
    table.set_targetname(first, NameId(3));
    table.set_targetname(other, NameId(4));
    table.set_targetname(last, NameId(3));
    let mut targets = TargetIndex::new(&table);
    assert!(targets.refresh(&table));
    assert!(!targets.refresh(&table));
    assert_eq!(targets.find(NameId(3)).collect::<Vec<_>>(), [first, last]);
    table.release(first, 0.0);
    assert!(targets.refresh(&table));
    assert_eq!(targets.find(NameId(3)).collect::<Vec<_>>(), [last]);
    let reused = table
        .allocate(0.0, ModuleId(0), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    table.set_targetname(reused, NameId(3));
    assert!(targets.refresh(&table));
    assert_eq!(targets.find(NameId(3)).collect::<Vec<_>>(), [reused, last]);
    table.set_targetname(last, NameId(4));
    assert!(targets.refresh(&table));
    assert_eq!(targets.find(NameId(4)).collect::<Vec<_>>(), [other, last]);
    assert!(!table.set_targetname(first, NameId(3)));
}
