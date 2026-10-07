use qa_core::primitives::{EntityId, ModuleId, Vec3};
use qa_world::entities::{EntityTable, MAX_ENTITIES, TableError};

#[test]
fn ed_alloc_delay_and_first_two_seconds_match_original_rules() {
    let mut table = EntityTable::new(3, 1).unwrap();
    let first = table.allocate(10.0, ModuleId(2)).unwrap();
    assert_eq!(first.slot, 1);
    table.columns.position[1] = Vec3([1.0, 2.0, 3.0]);
    assert!(table.release(first, 10.0));
    assert!(table.resolve(first).is_none());
    let second = table.allocate(10.5, ModuleId(3)).unwrap();
    assert_eq!(second.slot, 2);
    assert!(table.allocate(10.5, ModuleId(3)).is_none());
    let reused = table.allocate(10.500001, ModuleId(4)).unwrap();
    assert_eq!(reused.slot, first.slot);
    assert_ne!(reused.generation, first.generation);
    assert_eq!(table.columns.position[1], Vec3::default());
    assert_eq!(table.columns.owner[1], ModuleId(4));

    let mut startup = EntityTable::new(2, 1).unwrap();
    let entity = startup.allocate(1.0, ModuleId(0)).unwrap();
    assert!(startup.release(entity, 1.5));
    assert_eq!(
        startup.allocate(1.5, ModuleId(0)).unwrap().slot,
        entity.slot
    );
}

#[test]
fn stale_handles_and_reserved_slots_cannot_release_live_rows() {
    let mut table = EntityTable::new(5, 2).unwrap();
    let world = table.id_at(0).unwrap();
    assert!(!table.release(world, 0.0));
    let first = table.allocate(0.0, ModuleId(0)).unwrap();
    assert!(table.release(first, 0.0));
    let current = table.allocate(0.0, ModuleId(1)).unwrap();
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
