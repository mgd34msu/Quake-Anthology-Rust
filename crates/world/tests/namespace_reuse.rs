use qa_core::primitives::{ModuleId, ThinkTime};
use qa_world::entities::{AllocationPolicy, EntityTable, FullTable};

fn forced(id: qa_core::primitives::EntityId) -> AllocationPolicy {
    AllocationPolicy {
        full: FullTable::OverwriteOwned(id),
        ..AllocationPolicy::EDICT
    }
}

#[test]
fn selected_active_lifetime_is_replaced_even_with_spare_common_slots() {
    let mut table = EntityTable::new(8, 1).unwrap();
    let owner = ModuleId(1);
    let old = table
        .allocate(ThinkTime::Seconds(3.0), owner, AllocationPolicy::EDICT)
        .unwrap()
        .id;
    let untouched = table
        .allocate(ThinkTime::Seconds(3.0), owner, AllocationPolicy::EDICT)
        .unwrap()
        .id;
    let result = table
        .allocate(ThinkTime::Seconds(3.1), owner, forced(old))
        .unwrap();
    assert_eq!(result.displaced, Some(old));
    assert_eq!(result.id.slot, old.slot);
    assert_eq!(result.id.generation, old.generation + 1);
    assert!(table.resolve(old).is_none());
    assert!(table.resolve(untouched).is_some());
    assert_eq!(table.active().count(), 3);
}

#[test]
fn selected_retired_lifetime_ignores_delay_but_cannot_replace_a_reused_generation() {
    let mut table = EntityTable::new(4, 1).unwrap();
    let owner = ModuleId(1);
    let old = table
        .allocate(ThinkTime::Seconds(3.0), owner, AllocationPolicy::EDICT)
        .unwrap()
        .id;
    assert!(table.release(old, ThinkTime::Seconds(3.0)));
    let result = table
        .allocate(ThinkTime::Seconds(3.01), owner, forced(old))
        .unwrap();
    assert_eq!(result.displaced, None);
    assert_eq!(result.id.slot, old.slot);
    assert_eq!(result.id.generation, old.generation + 1);
    assert!(
        table
            .allocate(ThinkTime::Seconds(3.02), owner, forced(old))
            .is_none()
    );
    assert!(table.resolve(result.id).is_some());
}

#[test]
fn selected_reuse_rejects_foreign_and_protected_or_reserved_lifetimes() {
    let mut table = EntityTable::new(4, 1).unwrap();
    let owner = ModuleId(1);
    let old = table
        .allocate(ThinkTime::Seconds(3.0), owner, AllocationPolicy::EDICT)
        .unwrap()
        .id;
    assert!(table.set_never_free(old, true));
    assert!(
        table
            .allocate(ThinkTime::Seconds(3.0), owner, forced(old))
            .is_none()
    );
    assert!(table.set_never_free(old, false));
    assert!(
        table
            .allocate(ThinkTime::Seconds(3.0), ModuleId(2), forced(old))
            .is_none()
    );
    assert!(
        table
            .allocate(
                ThinkTime::Seconds(3.0),
                owner,
                forced(table.id_at(0).unwrap())
            )
            .is_none()
    );
    assert!(table.release(old, ThinkTime::Seconds(1.0)));
    let foreign = table
        .allocate(
            ThinkTime::Seconds(1.0),
            ModuleId(2),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    assert_eq!(foreign.slot, old.slot);
    assert_eq!(foreign.generation, old.generation + 1);
    assert!(
        table
            .allocate(ThinkTime::Seconds(3.0), owner, forced(old))
            .is_none()
    );
    assert!(table.resolve(foreign).is_some());
}
