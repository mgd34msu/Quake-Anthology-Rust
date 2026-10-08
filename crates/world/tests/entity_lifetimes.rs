use qa_core::primitives::{EntityId, ModuleId, Vec3};
use qa_world::entities::{AllocationPolicy, EntityTime};
use qa_world::entities::{EntityTable, MAX_ENTITIES, TableError};

#[test]
fn ed_alloc_delay_and_first_two_seconds_match_original_rules() {
    let mut table = EntityTable::new(3, 1).unwrap();
    let first = table
        .allocate(10.0, ModuleId(2), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    assert_eq!(first.slot, 1);
    table.columns.position[1] = Vec3([1.0, 2.0, 3.0]);
    assert!(table.release(first, 10.0));
    assert!(table.resolve(first).is_none());
    let second = table
        .allocate(10.5, ModuleId(3), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    assert_eq!(second.slot, 2);
    assert!(
        table
            .allocate(10.5, ModuleId(3), AllocationPolicy::EDICT)
            .is_none()
    );
    let reused = table
        .allocate(10.500001, ModuleId(4), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    assert_eq!(reused.slot, first.slot);
    assert_ne!(reused.generation, first.generation);
    assert_eq!(table.columns.position[1], Vec3::default());
    assert_eq!(table.columns.owner[1], ModuleId(4));

    let mut startup = EntityTable::new(2, 1).unwrap();
    let entity = startup
        .allocate(1.0, ModuleId(0), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    assert!(startup.release(entity, 1.5));
    assert_eq!(
        startup
            .allocate(1.5, ModuleId(0), AllocationPolicy::EDICT)
            .unwrap()
            .id
            .slot,
        entity.slot
    );
}

#[test]
fn stale_handles_and_reserved_slots_cannot_release_live_rows() {
    let mut table = EntityTable::new(5, 2).unwrap();
    let world = table.id_at(0).unwrap();
    assert!(!table.release(world, 0.0));
    let first = table
        .allocate(0.0, ModuleId(0), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    assert!(table.release(first, 0.0));
    let current = table
        .allocate(0.0, ModuleId(1), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    assert!(!table.release(first, 0.0));
    assert_eq!(table.resolve(current), Some(2));
    assert!(
        table
            .resolve(EntityId {
                slot: 99,
                generation: 1
            })
            .is_none()
    );
    assert_eq!(
        table.active().map(|id| id.slot).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(table.len(), 3);
    assert_eq!(table.capacity(), 5);
}

#[test]
fn map_capacity_is_bounded_at_load() {
    assert!(matches!(EntityTable::new(0, 0), Err(TableError::Capacity)));
    assert!(matches!(
        EntityTable::new(MAX_ENTITIES + 1, 0),
        Err(TableError::Capacity)
    ));
    assert!(matches!(
        EntityTable::new(1, 2),
        Err(TableError::ReservedSlots)
    ));
}

#[test]
fn q3_uses_integer_milliseconds_and_inclusive_native_boundaries() {
    let policy = AllocationPolicy::q3(10_000);
    let mut table = EntityTable::new(2, 1).unwrap();
    let first = table
        .allocate(EntityTime::Milliseconds(12_000), ModuleId(3), policy)
        .unwrap()
        .id;
    assert!(table.release(first, EntityTime::Milliseconds(12_000)));
    let early = table
        .allocate(EntityTime::Milliseconds(12_000), ModuleId(3), policy)
        .unwrap()
        .id;
    assert!(table.release(early, EntityTime::Milliseconds(12_001)));
    assert!(
        table
            .allocate(EntityTime::Milliseconds(13_000), ModuleId(3), policy)
            .is_none()
    );
    let exact = table
        .allocate(EntityTime::Milliseconds(13_001), ModuleId(3), policy)
        .unwrap()
        .id;
    assert_eq!(exact.slot, first.slot);
    assert_ne!(exact.generation, first.generation);
}

#[test]
fn edict_free_timestamp_has_native_binary32_precision() {
    let mut table = EntityTable::new(2, 1).unwrap();
    let first = table
        .allocate(3.0, ModuleId(1), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    assert!(table.release(first, 3.00000001));
    assert!(
        table
            .allocate(3.5, ModuleId(1), AllocationPolicy::EDICT)
            .is_none()
    );
    assert_eq!(
        table
            .allocate(3.500000001, ModuleId(1), AllocationPolicy::EDICT)
            .unwrap()
            .id
            .slot,
        first.slot
    );
}

#[test]
fn mixed_modules_retain_the_freed_lifetimes_reuse_rules() {
    let mut table = EntityTable::new(3, 1).unwrap();
    let q1 = table
        .allocate(10.0, ModuleId(1), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    let q3 = table
        .allocate(
            EntityTime::Milliseconds(10_000),
            ModuleId(3),
            AllocationPolicy::q3(0),
        )
        .unwrap()
        .id;
    assert!(table.release(q1, 10.0));
    assert!(table.release(q3, EntityTime::Milliseconds(10_000)));
    let reuse_q1 = table
        .allocate(10.6, ModuleId(3), AllocationPolicy::q3(0))
        .unwrap()
        .id;
    assert_eq!(reuse_q1.slot, q1.slot);
    assert!(
        table
            .allocate(10.6, ModuleId(1), AllocationPolicy::EDICT)
            .is_none()
    );
    let reuse_q3 = table
        .allocate(
            EntityTime::Milliseconds(11_000),
            ModuleId(1),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    assert_eq!(reuse_q3.slot, q3.slot);
    assert!(table.release(reuse_q3, 12.0));
    assert_eq!(
        table
            .allocate(12.6, ModuleId(1), AllocationPolicy::EDICT)
            .unwrap()
            .id
            .slot,
        q3.slot
    );
}

#[test]
fn qw_displaces_last_owned_lifetime_and_reports_spatial_unlink_handle() {
    use qa_core::primitives::Bounds;
    use qa_world::area::{AreaGrid, LinkFlags, LinkIntent, LinkOrder};
    let mut table = EntityTable::new(4, 1).unwrap();
    let first = table
        .allocate(10.0, ModuleId(1), AllocationPolicy::QUAKEWORLD)
        .unwrap()
        .id;
    let last = table
        .allocate(10.0, ModuleId(1), AllocationPolicy::QUAKEWORLD)
        .unwrap()
        .id;
    let foreign = table
        .allocate(10.0, ModuleId(3), AllocationPolicy::q3(0))
        .unwrap()
        .id;
    let mut area = AreaGrid::load(
        4,
        Bounds {
            mins: Vec3([-100.0; 3]),
            maxs: Vec3([100.0; 3]),
        },
    )
    .unwrap();
    assert!(area.link(
        &table,
        last,
        LinkFlags::SOLID,
        LinkOrder::Tail,
        LinkIntent::Explicit
    ));
    table.columns.frame[last.slot as usize] = 7;
    let replacement = table
        .allocate(10.0, ModuleId(1), AllocationPolicy::QUAKEWORLD)
        .unwrap();
    assert_eq!(replacement.displaced, Some(last));
    assert_eq!(replacement.id.slot, last.slot);
    assert_ne!(replacement.id.generation, last.generation);
    assert!(table.resolve(last).is_none());
    assert!(table.resolve(first).is_some());
    assert!(table.resolve(foreign).is_some());
    assert_eq!(table.columns.frame[replacement.id.slot as usize], 0);
    assert!(area.unlink(replacement.displaced.unwrap()));
    assert!(area.link(
        &table,
        replacement.id,
        LinkFlags::SOLID,
        LinkOrder::Head,
        LinkIntent::Explicit
    ));
    assert!(!area.unlink(last));
    assert_eq!(table.len(), 4);
}

#[test]
fn protected_body_queue_and_never_free_lifetimes_cannot_be_displaced() {
    let mut table = EntityTable::new(4, 2).unwrap();
    assert!(!table.release(table.id_at(1).unwrap(), 3.0));
    let first = table
        .allocate(3.0, ModuleId(1), AllocationPolicy::QUAKEWORLD)
        .unwrap()
        .id;
    let last = table
        .allocate(3.0, ModuleId(1), AllocationPolicy::QUAKEWORLD)
        .unwrap()
        .id;
    table.columns.frame[last.slot as usize] = 9;
    assert!(table.set_never_free(last, true));
    assert!(!table.release(last, 4.0));
    let replacement = table
        .allocate(4.0, ModuleId(1), AllocationPolicy::QUAKEWORLD)
        .unwrap();
    assert_eq!(replacement.displaced, Some(first));
    assert!(table.resolve(last).is_some());
    assert_eq!(table.columns.frame[last.slot as usize], 9);
    assert!(table.set_never_free(replacement.id, true));
    assert!(
        table
            .allocate(4.0, ModuleId(1), AllocationPolicy::QUAKEWORLD)
            .is_none()
    );
    assert!(table.set_never_free(last, false));
    assert!(table.release(last, 4.0));
    assert!(!table.set_never_free(last, true));
}
