use qa_core::primitives::{EntityId, ModuleId};
use qa_world::entities::{AllocationPolicy, EntityTable};

#[test]
fn ascending_lifetimes_cross_word_and_capacity_boundaries() -> Result<(), &'static str> {
    for capacity in [1, 63, 64, 65, 127, 128, 129, 8192] {
        let reserved = capacity.min(2);
        let mut table = EntityTable::new(capacity, reserved).map_err(|_| "table")?;
        let mut expected: Vec<_> = (0..reserved)
            .map(|slot| EntityId {
                slot: slot as u32,
                generation: 1,
            })
            .collect();
        for slot in reserved..capacity {
            let id = table
                .allocate(0.0, ModuleId(1), AllocationPolicy::EDICT)
                .ok_or("allocation")?
                .id;
            assert_eq!(id.slot as usize, slot);
            expected.push(id);
        }
        assert_eq!(table.active().collect::<Vec<_>>(), expected);
        assert_eq!(table.len(), capacity);
        assert!(
            table
                .allocate(0.0, ModuleId(1), AllocationPolicy::EDICT)
                .is_none()
        );
        expected.retain(|id| {
            if id.slot as usize >= reserved && id.slot % 3 == 1 {
                assert!(table.release(*id, 0.0));
                assert!(table.resolve(*id).is_none());
                false
            } else {
                true
            }
        });
        assert_eq!(table.active().collect::<Vec<_>>(), expected);
        assert_eq!(table.len(), expected.len());
        for start in [0, 1, 62, 63, 64, 65, 127, 128, capacity, usize::MAX] {
            assert_eq!(
                table.next_active(start),
                expected
                    .iter()
                    .copied()
                    .find(|id| id.slot as usize >= start)
            );
        }
        assert!(table.id_at(capacity).is_none());
        assert!(table.id_at(usize::MAX).is_none());
    }
    Ok(())
}

#[test]
fn reserved_prefix_and_never_free_do_not_become_allocatable() -> Result<(), &'static str> {
    let mut table = EntityTable::new(65, 64).map_err(|_| "table")?;
    let id = table
        .allocate(0.0, ModuleId(1), AllocationPolicy::QUAKEWORLD)
        .ok_or("entity")?
        .id;
    assert_eq!(id.slot, 64);
    assert!(table.set_never_free(id, true));
    assert!(!table.release(id, 0.0));
    assert!(
        table
            .allocate(0.0, ModuleId(1), AllocationPolicy::QUAKEWORLD)
            .is_none()
    );
    let world = table.id_at(0).ok_or("world")?;
    assert!(!table.release(world, 0.0));
    let client = table.id_at(63).ok_or("client")?;
    let reset = table.reset_client(client).ok_or("reset")?;
    assert_eq!(reset.slot, client.slot);
    assert_ne!(reset.generation, client.generation);
    assert!(table.resolve(client).is_none());
    assert!(table.resolve(reset).is_some());
    assert_eq!(table.len(), 65);
    assert!(table.set_never_free(id, false));
    assert!(table.release(id, 0.0));
    let reused = table
        .allocate(0.0, ModuleId(1), AllocationPolicy::EDICT)
        .ok_or("reuse")?
        .id;
    assert_eq!(reused.slot, 64);
    assert_ne!(reused.generation, id.generation);
    assert!(table.resolve(id).is_none());
    Ok(())
}
